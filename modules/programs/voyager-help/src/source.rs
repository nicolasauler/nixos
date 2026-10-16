use crate::layout::{Layout, parse_response};
use serde_json::json;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const SYSFS: &str = "/sys/bus/usb/devices";
const MAX_RESPONSE: u64 = 4 << 20;
const QUERY: &str = "query($hashId: String!, $revisionId: String!, $geometry: String) { \
    layout(hashId: $hashId, geometry: $geometry, revisionId: $revisionId) { \
    hashId revision { hashId title layers { title keys } } } }";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub layout: String,
    pub revision: String,
}

fn parse_identity(serial: &str) -> Result<Identity, String> {
    // sysfs appends one newline; other whitespace is not part of an Oryx ID.
    let serial = serial.strip_suffix('\n').unwrap_or(serial);
    let (layout, revision) = serial
        .split_once('/')
        .ok_or("Voyager USB serial must report an Oryx layout/revision")?;
    let valid = |id: &str| {
        !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    };
    if !valid(layout) || !valid(revision) || revision.eq_ignore_ascii_case("latest") {
        return Err(
            "Voyager USB serial has an invalid Oryx layout/revision; an exact revision is required"
                .into(),
        );
    }
    Ok(Identity {
        layout: layout.into(),
        revision: revision.into(),
    })
}

fn read_limited(reader: impl Read, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    read_limited(File::open(path)?, limit)
}

fn metadata(path: &Path) -> io::Result<String> {
    String::from_utf8(read_bounded(path, 256)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn usb_id(path: &Path, expected: &str) -> Result<bool, String> {
    match metadata(path) {
        Ok(value) => Ok(value.trim_end_matches('\n').eq_ignore_ascii_case(expected)),
        // USB interface entries have no device IDs; devices can also disappear.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Cannot read USB metadata {}: {error}",
            path.display()
        )),
    }
}

fn discover(root: &Path) -> Result<Identity, String> {
    let devices = fs::read_dir(root)
        .map_err(|error| format!("Cannot inspect USB metadata {}: {error}", root.display()))?;
    let mut identity: Option<Identity> = None;
    for device in devices {
        let path = device
            .map_err(|error| format!("Cannot enumerate USB metadata: {error}"))?
            .path();
        if !usb_id(&path.join("idVendor"), "3297")? || !usb_id(&path.join("idProduct"), "1977")? {
            continue;
        }
        let serial_path = path.join("serial");
        let serial = metadata(&serial_path).map_err(|error| {
            format!(
                "Cannot read Voyager Oryx identity {}: {error}",
                serial_path.display()
            )
        })?;
        let found = parse_identity(&serial)
            .map_err(|error| format!("{}: {error}", serial_path.display()))?;
        if let Some(previous) = &identity {
            if previous != &found {
                return Err(format!(
                    "Ambiguous Voyagers: connected keyboards report different Oryx identities ({}/{} and {}/{}); connect only the intended keyboard",
                    previous.layout, previous.revision, found.layout, found.revision
                ));
            }
        } else {
            identity = Some(found);
        }
    }
    identity.ok_or_else(|| "No connected Voyager (USB 3297:1977) found; refusing to show a cached layout without a keyboard".into())
}

fn cache_root() -> Result<PathBuf, String> {
    if let Some(root) = env::var_os("XDG_CACHE_HOME") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return Ok(root.join("voyager-help"));
        }
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or("Set an absolute XDG_CACHE_HOME or HOME for the Voyager reference cache")?;
    Ok(home.join(".cache/voyager-help"))
}

fn cache_path(root: &Path, identity: &Identity) -> PathBuf {
    root.join(format!("{}-{}.json", identity.layout, identity.revision))
}

fn publish(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path.parent().ok_or("Cache path has no parent directory")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Cannot create cache {}: {error}", directory.display()))?;
    let (temporary, mut file) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(
            ".voyager-help-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot create cache temporary file: {error}")),
        }
    };
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("Cannot publish cache {}: {error}", path.display()))
}

fn fetch(identity: &Identity) -> Result<Vec<u8>, String> {
    let request = json!({
        "query": QUERY,
        "variables": {
            "hashId": identity.layout,
            "revisionId": identity.revision,
            "geometry": "voyager"
        }
    });
    // Match the firmware updater's ureq request deadline. This bounds normal
    // HTTP waits, not a hard process deadline against trickling TLS records.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent
        .post("https://oryx.zsa.io/graphql")
        .header("Content-Type", "application/json")
        .send(request.to_string())
        .map_err(|error| format!("Oryx GraphQL request failed: {error}"))?;
    response_bytes(response.body_mut(), MAX_RESPONSE)
}

fn response_bytes(body: &mut ureq::Body, limit: u64) -> Result<Vec<u8>, String> {
    // ureq limits encoded bytes before decompression. Bound the decoded stream
    // separately; the extra byte probes overflow while accepting exactly limit.
    read_limited(body.with_config().limit(limit + 1).reader(), limit)
        .map_err(|error| format!("Oryx GraphQL response failed: {error}"))
}

fn load_from(
    sysfs: &Path,
    cache: &Path,
    fetcher: impl FnOnce(&Identity) -> Result<Vec<u8>, String>,
) -> Result<Layout, String> {
    let identity = discover(sysfs)?;
    let path = cache_path(cache, &identity);
    let cached = read_bounded(&path, MAX_RESPONSE)
        .map_err(|error| format!("Cache {}: {error}", path.display()))
        .and_then(|bytes| parse_response(&bytes, &identity.layout, &identity.revision));
    let layout = match cached {
        Ok(layout) => layout,
        Err(cache_error) => {
            let bytes = fetcher(&identity).map_err(|error| format!(
                "No valid cached reference for {}/{} ({cache_error}); {error}. Connect to the network to retrieve this exact revision",
                identity.layout, identity.revision
            ))?;
            if bytes.len() as u64 > MAX_RESPONSE {
                return Err("Oryx GraphQL response exceeds the 4 MiB size limit".into());
            }
            let layout =
                parse_response(&bytes, &identity.layout, &identity.revision).map_err(|error| {
                    format!(
                        "Invalid Oryx response for {}/{}: {error}",
                        identity.layout, identity.revision
                    )
                })?;
            publish(&path, &bytes)?;
            layout
        }
    };
    let current = discover(sysfs).map_err(|error| {
        format!("Voyager identity could not be rechecked; no reference will be shown: {error}")
    })?;
    if current != identity {
        return Err(format!(
            "Voyager changed while loading the reference ({}/{} -> {}/{}); open the reference again",
            identity.layout, identity.revision, current.layout, current.revision
        ));
    }
    Ok(layout)
}

pub fn load() -> Result<Layout, String> {
    load_from(Path::new(SYSFS), &cache_root()?, fetch)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = loop {
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = env::temp_dir().join(format!(
                    "voyager-source-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => break path,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("Cannot create private test directory: {error}"),
                }
            };
            fs::create_dir(root.join("usb")).unwrap();
            fs::create_dir(root.join("cache")).unwrap();
            Self(root)
        }

        fn usb(&self) -> PathBuf {
            self.0.join("usb")
        }
        fn cache(&self) -> PathBuf {
            self.0.join("cache")
        }

        fn device(&self, name: &str, serial: &str) {
            let path = self.usb().join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("idVendor"), "3297\n").unwrap();
            fs::write(path.join("idProduct"), "1977\n").unwrap();
            fs::write(path.join("serial"), serial).unwrap();
        }

        fn load(
            &self,
            fetcher: impl FnOnce(&Identity) -> Result<Vec<u8>, String>,
        ) -> Result<Layout, String> {
            load_from(&self.usb(), &self.cache(), fetcher)
        }

        fn cached(&self, serial: &str, bytes: &[u8]) -> PathBuf {
            let path = cache_path(&self.cache(), &parse_identity(serial).unwrap());
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn response(layout: &str, revision: &str) -> Vec<u8> {
        let key = json!({"tap": null, "hold": null, "doubleTap": null, "tapHold": null});
        serde_json::to_vec(&json!({"data": {"layout": {
            "hashId": layout,
            "revision": {"hashId": revision, "title": "Test reference", "layers": [
                {"title": "Main", "keys": vec![key; 52]}
            ]}
        }}}))
        .unwrap()
    }

    #[test]
    fn serial_requires_safe_exact_identifiers() {
        assert_eq!(
            parse_identity("JqpmQ/bvrAW7\n").unwrap(),
            Identity {
                layout: "JqpmQ".into(),
                revision: "bvrAW7".into()
            }
        );
        for serial in [
            "",
            "layout",
            "/rev",
            "layout/",
            "../rev",
            "layout/../rev",
            "layout/rev/extra",
            "layout/latest",
            "layout/LATEST",
            "layout/rev?x",
            "layout/rev\n\n",
            " layout/rev",
            "layout/rév",
            "layout/rev\0",
        ] {
            assert!(parse_identity(serial).is_err(), "Accepted {serial:?}");
        }
        assert!(parse_identity(&format!("layout/{}", "a".repeat(65))).is_err());
    }

    #[test]
    fn no_device_never_uses_stale_cache() {
        let fixture = Fixture::new();
        fixture.cached("JqpmQ/old", &response("JqpmQ", "old"));
        assert!(
            fixture
                .load(|_| panic!("No device must not fetch"))
                .is_err()
        );
    }

    #[test]
    fn unrelated_usb_devices_and_interface_entries_are_ignored() {
        let fixture = Fixture::new();
        fixture.device("other", "not-an-oryx-serial");
        fs::write(fixture.usb().join("other/idProduct"), "1969\n").unwrap();
        fs::create_dir(fixture.usb().join("1-1:1.0")).unwrap();
        assert!(
            fixture
                .load(|_| panic!("No Voyager must not fetch"))
                .is_err()
        );
    }

    #[test]
    fn malformed_or_missing_serial_never_fetches() {
        let fixture = Fixture::new();
        fixture.device("1-1", "JqpmQ/latest\n");
        assert!(
            fixture
                .load(|_| panic!("Invalid identity must not fetch"))
                .is_err()
        );
        fs::remove_file(fixture.usb().join("1-1/serial")).unwrap();
        assert!(
            fixture
                .load(|_| panic!("Missing identity must not fetch"))
                .is_err()
        );
    }

    #[test]
    fn distinct_revisions_or_layouts_are_ambiguous_but_duplicates_are_allowed() {
        let fixture = Fixture::new();
        fixture.device("1-1", "JqpmQ/revA\n");
        fixture.device("1-2", "JqpmQ/revB\n");
        assert!(
            fixture
                .load(|_| panic!("Ambiguous identity must not fetch"))
                .is_err()
        );
        fixture.device("1-2", "Other/revA\n");
        assert!(discover(&fixture.usb()).is_err());
        fixture.device("1-2", "JqpmQ/revA\n");
        assert_eq!(
            discover(&fixture.usb()).unwrap(),
            parse_identity("JqpmQ/revA").unwrap()
        );
    }

    #[test]
    fn exact_cache_hit_works_offline_and_is_not_rewritten() {
        let fixture = Fixture::new();
        fixture.device("1-1", "JqpmQ/revA\n");
        let bytes = response("JqpmQ", "revA");
        let path = fixture.cached("JqpmQ/revA", &bytes);
        let layout = fixture
            .load(|_| panic!("Valid cache must not use the network"))
            .unwrap();
        assert_eq!(layout.revision, "revA");
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn stale_revision_is_not_an_offline_fallback() {
        let fixture = Fixture::new();
        fixture.device("1-1", "JqpmQ/revB\n");
        let bytes = response("JqpmQ", "revA");
        let old = fixture.cached("JqpmQ/revA", &bytes);
        let result = fixture.load(|identity| {
            assert_eq!(identity, &parse_identity("JqpmQ/revB").unwrap());
            Err("offline".into())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(old).unwrap(), bytes);
        assert!(!cache_path(&fixture.cache(), &parse_identity("JqpmQ/revB").unwrap()).exists());
    }

    #[test]
    fn both_cached_identities_are_validated_and_invalid_cache_is_refetched() {
        for (layout, revision) in [("Other", "revB"), ("JqpmQ", "revA")] {
            let fixture = Fixture::new();
            fixture.device("1-1", "JqpmQ/revB\n");
            let path = fixture.cached("JqpmQ/revB", &response(layout, revision));
            let bytes = response("JqpmQ", "revB");
            let loaded = fixture.load(|_| Ok(bytes.clone())).unwrap();
            assert_eq!(loaded.layout, "JqpmQ");
            assert_eq!(loaded.revision, "revB");
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn wrong_or_malformed_fetched_responses_are_never_cached() {
        for bytes in [
            response("Other", "revB"),
            response("JqpmQ", "revA"),
            b"{truncated".to_vec(),
            br#"{"data":{"layout":null}}"#.to_vec(),
        ] {
            let fixture = Fixture::new();
            fixture.device("1-1", "JqpmQ/revB\n");
            assert!(fixture.load(|_| Ok(bytes)).is_err());
            assert_eq!(fs::read_dir(fixture.cache()).unwrap().count(), 0);
        }
    }

    #[test]
    fn interrupted_or_malformed_fetch_preserves_concurrently_published_good_cache() {
        for malformed in [false, true] {
            let fixture = Fixture::new();
            fixture.device("1-1", "JqpmQ/revB\n");
            let good = response("JqpmQ", "revB");
            let result = fixture.load(|_| {
                // A second invocation completes while this invocation is fetching.
                fixture.cached("JqpmQ/revB", &good);
                if malformed {
                    Ok(b"{truncated".to_vec())
                } else {
                    Err("interrupted response".into())
                }
            });
            assert!(result.is_err());
            let path = cache_path(&fixture.cache(), &parse_identity("JqpmQ/revB").unwrap());
            assert_eq!(fs::read(path).unwrap(), good);
            assert_eq!(
                fixture
                    .load(|_| panic!("Good cache must remain usable offline"))
                    .unwrap()
                    .revision,
                "revB"
            );
        }
    }

    #[test]
    fn changed_or_disconnected_board_is_rejected_after_fetch() {
        for disconnect in [false, true] {
            let fixture = Fixture::new();
            fixture.device("1-1", "JqpmQ/revA\n");
            let result = fixture.load(|_| {
                if disconnect {
                    fs::remove_dir_all(fixture.usb().join("1-1")).unwrap();
                } else {
                    fixture.device("1-1", "JqpmQ/revB\n");
                }
                Ok(response("JqpmQ", "revA"))
            });
            assert!(result.is_err());
        }
    }

    #[test]
    fn oversized_cache_is_refetched_and_oversized_response_is_rejected() {
        let fixture = Fixture::new();
        fixture.device("1-1", "JqpmQ/revA\n");
        let mut too_large = response("JqpmQ", "revA");
        too_large.resize(MAX_RESPONSE as usize + 1, b' ');
        let path = fixture.cached("JqpmQ/revA", &too_large);
        let bytes = response("JqpmQ", "revA");
        fixture.load(|_| Ok(bytes.clone())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_file(&path).unwrap();
        assert!(fixture.load(|_| Ok(too_large)).is_err());
        assert!(!path.exists());
    }

    fn http_body(data: Vec<u8>, encoding: &str) -> ureq::Body {
        use std::net::TcpListener;
        use std::thread;
        use std::time::Instant;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Encoding: {encoding}\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "test client did not connect");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("test listener: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(&data).unwrap();
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        let response = agent.get(format!("http://{address}/")).call().unwrap();
        server.join().unwrap();
        response.into_body()
    }

    fn gzip_spaces(length: u64) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        io::copy(&mut io::repeat(b' ').take(length), &mut encoder).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn gzip_decoded_body_respects_the_same_inclusive_limit() {
        let limit = 64;
        let mut expanded = http_body(gzip_spaces(8192), "gzip");
        let result = response_bytes(&mut expanded, limit);
        assert!(
            result.is_err(),
            "compressed body expanded to {} bytes before rejection",
            result.unwrap().len()
        );
        for encoding in ["identity", "gzip"] {
            for length in [limit, limit + 1] {
                let data = if encoding == "gzip" {
                    gzip_spaces(length)
                } else {
                    vec![b' '; length as usize]
                };
                let mut body = http_body(data, encoding);
                let result = response_bytes(&mut body, limit);
                if length == limit {
                    assert_eq!(result.unwrap(), vec![b' '; limit as usize]);
                } else {
                    assert!(result.is_err());
                }
            }
        }
    }

    #[test]
    fn decoded_overflow_stops_reading_at_the_boundary() {
        let mut body = http_body(gzip_spaces(8192), "gzip");
        let mut reader = body.with_config().limit(65).reader();
        assert!(read_limited(&mut reader, 64).is_err());
        let mut remaining = Vec::new();
        reader.read_to_end(&mut remaining).unwrap();
        assert_eq!(remaining, vec![b' '; 8192 - 65]);
    }
}
