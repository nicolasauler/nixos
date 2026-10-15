//! Hyprland's socket1 `locked` command reads the session-lock protocol's
//! authoritative state, including a locker that dies before rendering. Unlike
//! lock-notify, it does not wait for the first frame. Each query has one 250 ms
//! deadline across nonblocking connect, write and read; errors remain unknown.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::libc;

use crate::machine::Lock;

pub const WAIT: Duration = Duration::from_millis(250);
const REQUEST: &[u8] = b"/locked";

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Hyprland lock query timed out"))
}

fn ready(stream: &UnixStream, events: libc::c_short, deadline: Instant) -> io::Result<()> {
    loop {
        let left = remaining(deadline)?;
        let timeout = libc::timespec {
            tv_sec: left.as_secs() as libc::time_t,
            tv_nsec: left.subsec_nanos() as libc::c_long,
        };
        let mut fd = libc::pollfd {
            fd: stream.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: fd and timeout are initialized and valid for this call;
        // a null signal mask leaves the calling thread's mask unchanged.
        let result = unsafe { libc::ppoll(&mut fd, 1, &timeout, std::ptr::null()) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        remaining(deadline)?;
        if result > 0 {
            if fd.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::from_raw_os_error(libc::EBADF));
            }
            // HUP/ERR also wake the nonblocking operation, which reports EOF
            // or the socket error rather than waiting again.
            return Ok(());
        }
    }
}

fn connect(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    // SAFETY: all-zero is a valid initial sockaddr_un representation.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Hyprland socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (to, from) in address.sun_path.iter_mut().zip(bytes) {
        *to = *from as libc::c_char;
    }
    remaining(deadline)?;
    // SAFETY: socket takes no pointers. NONBLOCK applies from creation, so
    // even a full Unix-socket accept queue cannot block connect.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh, uniquely owned Unix stream socket.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    remaining(deadline)?;
    // SAFETY: address contains a terminated path and length includes only
    // initialized bytes within that sockaddr_un.
    let result = unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
        ready(&stream, libc::POLLOUT, deadline)?;
        if let Some(error) = stream.take_error()? {
            return Err(error);
        }
    }
    Ok(stream)
}

fn query(path: &Path, deadline: Instant) -> io::Result<Lock> {
    let mut stream = connect(path, deadline)?;
    let mut sent = 0;
    while sent < REQUEST.len() {
        remaining(deadline)?;
        match stream.write(&REQUEST[sent..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "empty lock request write",
                ));
            }
            Ok(size) => sent += size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                ready(&stream, libc::POLLOUT, deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    // Socket1 closes after its reply. Read through EOF, not just a valid
    // prefix: "false garbage" must never authorize a new engagement.
    let mut response = [0; 6];
    let mut used = 0;
    loop {
        remaining(deadline)?;
        match stream.read(&mut response[used..]) {
            Ok(0) => break,
            Ok(size) => {
                used += size;
                if used == response.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "oversized lock response",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                ready(&stream, libc::POLLIN, deadline)?;
            }
            Err(error) => return Err(error),
        }
    }
    remaining(deadline)?;
    match &response[..used] {
        b"true" => Ok(Lock::Locked),
        b"false" => Ok(Lock::Unlocked),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid lock response",
        )),
    }
}

/// Read the authoritative lock state once; callers must fail closed on error.
pub fn now() -> Result<Lock, String> {
    let deadline = Instant::now() + WAIT;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
    if runtime.is_empty() || signature.is_empty() {
        return Err("Hyprland runtime directory or instance signature is empty".to_owned());
    }
    let path = PathBuf::from(runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    query(&path, deadline).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    struct SocketPath(PathBuf);

    impl SocketPath {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "talkd-lock-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir.join("socket"))
        }
    }

    impl Drop for SocketPath {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
        }
    }

    fn request(stream: &mut UnixStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut bytes = [0; REQUEST.len()];
        stream.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, REQUEST);
    }

    fn reply(bytes: &'static [u8]) -> io::Result<Lock> {
        let path = SocketPath::new();
        let listener = UnixListener::bind(&path.0).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request(&mut stream);
            stream.write_all(bytes).unwrap();
        });
        let result = query(&path.0, Instant::now() + WAIT);
        server.join().unwrap();
        result
    }

    #[test]
    fn authoritative_true_and_false() {
        assert_eq!(reply(b"true").unwrap(), Lock::Locked);
        assert_eq!(reply(b"false").unwrap(), Lock::Unlocked);
    }

    #[test]
    fn malformed_and_oversized_replies_are_unknown() {
        for bytes in [
            b"".as_slice(),
            b"tru",
            b"False",
            b"false\n",
            b"false garbage",
        ] {
            assert_eq!(reply(bytes).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn missing_socket_is_unknown() {
        let path = SocketPath::new();
        assert_eq!(
            query(&path.0, Instant::now() + WAIT).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn stalled_response_uses_one_deadline() {
        let path = SocketPath::new();
        let listener = UnixListener::bind(&path.0).unwrap();
        let (received, request_received) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            request(&mut stream);
            // Even an otherwise valid prefix must await EOF within the
            // original deadline; the server stays open until explicitly freed.
            stream.write_all(b"false").unwrap();
            received.send(()).unwrap();
            let _ = released.recv();
        });
        let socket = path.0.clone();
        let (completed, completion) = mpsc::channel();
        let client = thread::spawn(move || {
            let started = Instant::now();
            let result = query(&socket, started + WAIT);
            completed.send((result, started.elapsed())).unwrap();
        });
        let received = request_received.recv_timeout(Duration::from_secs(1));
        let result = completion.recv_timeout(Duration::from_secs(1));
        release.send(()).unwrap();
        server.join().unwrap();
        client.join().unwrap();
        received.unwrap();
        let (result, elapsed) = result.unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(elapsed >= WAIT);
        // Permit scheduler delay, but not the old unbounded compositor wait.
        assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    }
}
