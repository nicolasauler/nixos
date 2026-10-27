# Behavioural proof of modules/services/fingerprint.nix: a fingerprint unlocks
# polkit-1 and nothing else.
#
# Why a VM and not an eval assertion. Reading `fprintAuth` back from the module
# system proves what was asked for, not what PAM does: that pam_fprintd is
# actually reached in the polkit-1 stack, that fprintd answers it over D-Bus for
# a user other than the caller, that a matching print is `sufficient` on its own
# (no password conversation), and that a non-matching one falls through to the
# password instead of being accepted. The reader is libfprint's virtual driver
# (`FP_VIRTUAL_DEVICE`): fprintd opens a Unix socket at that path while the device
# is claimed, and each connection carries one command, `SCAN <id>` being a finger
# with that identity. Stock fprintd, not the TOD build the laptop runs: only
# libfprint proper carries the virtual drivers (measured: `strings` on
# libfprint-2-tod.so.1 has none). The Dell driver itself is hardware and cannot be
# exercised here.
#
# The control (wrong finger, three times) is what gives the first subtest meaning:
# without it, "successfully authenticated" could just as well be pam_permit, an
# empty password, or pam_fprintd being skipped.
#
# The last subtest is deliberately a file check: it is the invariant the module
# exists for -- polkit-1 is the ONLY service whose stack names pam_fprintd -- and
# the subtests before it are what make "names pam_fprintd" equivalent to "asks for
# a finger first".
#
# No private input and no credential: the test user has no password (the
# fallthrough is meant to fail), so this is safe for the public CI job.
{pkgs, ...}: let
  user = "nic";
  # /var/lib/fprint is fprintd's StateDirectory -- the one path its
  # ProtectSystem=strict unit can create a socket in.
  socket = "/var/lib/fprint/virtual-reader.sock";
  finger = "${user}-right-index";
in
  pkgs.testers.runNixOSTest {
    name = "fingerprint-pam";

    nodes.machine = {pkgs, ...}: {
      imports = [
        ../modules/services/fingerprint.nix
      ];

      # fprintd authorises callers through polkit; the laptop gets polkitd from
      # its desktop stack, this node has to say so.
      security.polkit.enable = true;

      systemd.services.fprintd.environment.FP_VIRTUAL_DEVICE = socket;

      # pam_fprintd's debug log is how the test knows its VerifyStart call has
      # returned; see touch_when_verifying below.
      security.pam.services.polkit-1.rules.auth.fprintd.settings.debug = true;

      users.users.${user}.isNormalUser = true;

      environment.systemPackages = [
        pkgs.pamtester
        pkgs.python3
      ];

      # One command per connection; the socket only exists while a client holds
      # the device, so retry until fprintd is listening rather than racing it.
      # Optional second argument: seconds to keep trying (default 30).
      environment.etc."virtual-reader.py".text = ''
        import socket, sys, time

        deadline = time.monotonic() + (float(sys.argv[2]) if len(sys.argv) > 2 else 30)
        while True:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect("${socket}")
                s.sendall(sys.argv[1].encode())
                break
            except OSError as e:
                if time.monotonic() > deadline:
                    raise SystemExit(f"virtual reader not reachable: {e}")
                time.sleep(0.1)
            finally:
                s.close()
      '';
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")


      def touch(finger_id):
          machine.succeed(f"python3 /etc/virtual-reader.py 'SCAN {finger_id}'")


      def in_background(cmd, log):
          machine.succeed(f"{cmd} </dev/null >{log} 2>&1 & echo $! >{log}.pid")


      def wait_exit(log):
          # the launching shell is gone, so init reaps the process on exit
          machine.wait_until_succeeds(f"! kill -0 $(cat {log}.pid) 2>/dev/null", timeout=120)


      def verify_starts():
          return int(machine.succeed(
              "journalctl -b -o cat | grep -c 'VerifyStart completed successfully' || true"
          ))


      def touch_when_verifying(finger_id, seen):
          # A result that reaches pam_fprintd before its VerifyStart call has
          # returned is dropped ("Unexpected VerifyResult"), and pam_fprintd then
          # waits out its timeout and falls through to the password. So present the
          # finger only once the next VerifyStart has completed.
          machine.wait_until_succeeds(
              "[ $(journalctl -b -o cat | grep -c 'VerifyStart completed successfully')"
              f" -gt {seen} ]",
              timeout=60,
          )
          touch(finger_id)
          return seen + 1


      with subtest("enroll a finger for ${user} on the virtual reader"):
          in_background("fprintd-enroll -f right-index-finger ${user}", "/tmp/enroll.log")
          # the virtual device's default is five enroll stages
          for _ in range(5):
              touch("${finger}")
          wait_exit("/tmp/enroll.log")
          machine.succeed("grep -q 'enroll-completed' /tmp/enroll.log")
          machine.succeed("fprintd-list ${user} | grep -q right-index-finger")

      with subtest("polkit-1 accepts the enrolled finger with no password conversation"):
          # stdin is /dev/null: any password prompt would be a conversation
          # error, so success here can only have come from pam_fprintd
          seen = verify_starts()
          in_background("pamtester polkit-1 ${user} authenticate", "/tmp/pam-match.log")
          touch_when_verifying("${finger}", seen)
          wait_exit("/tmp/pam-match.log")
          machine.succeed("grep -q 'successfully authenticated' /tmp/pam-match.log")

      with subtest("control: a wrong finger falls through to the password and fails"):
          seen = verify_starts()
          in_background("pamtester polkit-1 ${user} authenticate", "/tmp/pam-nomatch.log")
          # pam_fprintd re-arms the reader after each no-match, up to its max-tries
          # (3 by default), and only then falls through to the password.
          for _ in range(3):
              seen = touch_when_verifying("someone-else", seen)
          wait_exit("/tmp/pam-nomatch.log")
          # all three attempts were rejected as mismatches: none ran into the
          # timeout, as one would if its scan went missing
          machine.succeed("[ $(grep -c 'Failed to match fingerprint' /tmp/pam-nomatch.log) -eq 3 ]")
          machine.fail("grep -q 'Verification timed out' /tmp/pam-nomatch.log")
          machine.fail("grep -q 'successfully authenticated' /tmp/pam-nomatch.log")

      with subtest("no other PAM service names pam_fprintd"):
          stacks = machine.succeed("grep -l pam_fprintd.so /etc/pam.d/* || true").split()
          assert stacks == ["/etc/pam.d/polkit-1"], stacks
    '';
  }
