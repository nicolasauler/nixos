# Behavioural proof that `max-substitution-jobs` in the CI daemon's own
# environment bounds anything.
#
# Why this test exists. The whole reason the CI daemon carries a NIX_CONFIG of
# its own is that `max-substitution-jobs` is unreachable from the client: it
# travels in the trust-gated `overrides` map (nix src/libstore/daemon.cc:296-308),
# unlike `cores`/`max-jobs`, which `ClientSettings::apply` assigns
# unconditionally (daemon.cc:240,243). So an untrusted `buildbot-worker` cannot
# lower it and sixteen concurrent NAR fetch-and-decompress jobs
# (worker-settings.hh:73-76) run unbounded in whichever daemon serves it.
#
# Asserting that the unit carries the string `max-substitution-jobs = 4` proves
# only that the string is there. It does not prove nix reads NIX_CONFIG in a
# daemon process, that the value survives `ClientSettings::apply` for an
# untrusted connection, or that anything is actually serialised. This measures
# it: a local binary cache is served by an HTTP server that sleeps 3s on every
# `/nar/` GET and records the peak number of overlapping ones, and eight paths
# are fetched through each daemon.
#
# The two phases are an exact A/B — same eight paths, same cache, same untrusted
# client, same command. Only `NIX_REMOTE` differs:
#   * unix:///run/nix-daemon-ci/socket -> peak must be exactly 4
#   * daemon (the system instance)     -> peak must exceed 4
# The control is what makes the first number mean something: without it, a peak
# of 4 could just as well be the transport, the disk, or the VM's core count
# serialising the fetches.
#
# No private input: this node imports only the CI daemon module, so CI can run it
# on a hosted runner (see .github/workflows/ci.yaml).
{pkgs, ...}: let
  cacheDir = "/tmp/binary-cache";
  cachePort = 18080;
  cacheUrl = "http://127.0.0.1:${toString cachePort}";
  # unprivileged, and deliberately NOT in trusted-users: the point is what a
  # client that cannot set restricted settings gets.
  ciUser = "buildbot-worker";
  payloads = 8;
  expectedPeak = 4;
in
  pkgs.testers.runNixOSTest {
    name = "nix-substitution-limit";

    nodes.machine = {
      lib,
      pkgs,
      ...
    }: {
      imports = [
        ../modules/services/nix-daemon-ci.nix
      ];

      virtualisation = {
        # more cores than the cap, so a peak of 4 cannot be the core count
        cores = 8;
        memorySize = 4096;
        diskSize = 8192;
      };

      users.groups.${ciUser} = {};
      users.users.${ciUser} = {
        isSystemUser = true;
        group = ciUser;
        home = "/var/lib/${ciUser}";
        createHome = true;
      };

      services.nixDaemonCi = {
        enable = true;
        group = ciUser;
        # Stated here rather than inherited, because it is the value under test.
        # Note this is NOT the module's whole default: nix-daemon-ci.nix:58-62
        # also zeroes min-free/max-free (inert here — nix's own default for both
        # is already 0). Consequence worth knowing: because this node sets
        # nixConfig explicitly, it never reads the module default, so raising
        # THAT to 16 leaves this check green. The string assertion in
        # buildbot-workstation.nix:191 is what catches that. The two are
        # complementary: this one proves the mechanism works, that one proves
        # desktop is actually configured with it.
        nixConfig = ''
          max-substitution-jobs = ${toString expectedPeak}
        '';
      };

      nix.settings = {
        experimental-features = ["nix-command" "flakes"];
        # only the local cache: mkForce so cache.nixos.org is not even tried,
        # which in an offline VM only buys five DNS retries per fetch
        substituters = lib.mkForce [cacheUrl];
        # the local cache is unsigned
        require-sigs = false;
        # Not desktop's literal list (that is ["nic"], hosts/desktop/
        # configuration.nix:425-427) — what matters is the property both share:
        # the CI identity is absent from it, so buildbot-worker connects as an
        # untrusted client and cannot raise the cap itself.
        trusted-users = ["root"];
      };

      environment.systemPackages = [pkgs.python3];

      # Counts overlapping NAR GETs and holds each one open long enough for the
      # overlap to be observable. Peak is written on every change so the test can
      # read it after the fetch returns.
      environment.etc."slow-cache.py".text = ''
        import http.server
        import os
        import threading
        import time

        lock = threading.Lock()
        active = 0
        peak = 0


        def write_state():
            with open("/tmp/cache-peak.tmp", "w") as f:
                f.write(str(peak))
            os.replace("/tmp/cache-peak.tmp", "/tmp/cache-peak")


        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, fmt, *args):
                pass

            def do_GET(self):
                global active, peak
                delayed = "/nar/" in self.path
                if delayed:
                    with lock:
                        active += 1
                        peak = max(peak, active)
                        write_state()
                    time.sleep(3)
                try:
                    super().do_GET()
                finally:
                    if delayed:
                        with lock:
                            active -= 1


        os.chdir("${cacheDir}")
        write_state()
        http.server.ThreadingHTTPServer(("127.0.0.1", ${toString cachePort}), Handler).serve_forever()
      '';
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target")
      machine.wait_for_unit("nix-daemon-ci.socket")

      # Eight store paths, pushed to a local cache, then removed from the store
      # so they have to be substituted back.
      paths = machine.succeed("""
        set -eu
        rm -rf ${cacheDir} /tmp/substitution-paths
        mkdir -p ${cacheDir}
        : >/tmp/substitution-paths
        for i in $(seq 1 ${toString payloads}); do
          printf 'payload-%s-%0100000d' "$i" 0 >"/tmp/payload-$i"
          nix store add-file "/tmp/payload-$i" >>/tmp/substitution-paths
        done
        nix copy --to 'file://${cacheDir}?compression=xz' $(cat /tmp/substitution-paths)
        cat /tmp/substitution-paths
      """).split()
      assert len(paths) == ${toString payloads}, paths


      def start_cache():
          machine.succeed("rm -f /tmp/cache-peak")
          machine.succeed(
              "python3 /etc/slow-cache.py >/tmp/slow-cache.log 2>&1 & "
              "echo $! >/tmp/slow-cache.pid"
          )
          machine.wait_until_succeeds("curl -sf ${cacheUrl}/nix-cache-info")


      def stop_cache():
          machine.succeed("kill $(cat /tmp/slow-cache.pid)")


      def substitute_through(remote):
          """Fetch all eight paths as the untrusted CI user, return the peak
          number of NAR GETs the cache saw overlapping."""
          machine.succeed("nix store delete " + " ".join(paths))
          start_cache()
          try:
              machine.succeed(
                  "runuser -u ${ciUser} -- env HOME=/var/lib/${ciUser} "
                  f"NIX_REMOTE={remote} nix-store -r " + " ".join(paths)
              )
              return int(machine.succeed("cat /tmp/cache-peak").strip())
          finally:
              stop_cache()


      with subtest("the CI daemon holds substitution to max-substitution-jobs"):
          peak = substitute_through("unix:///run/nix-daemon-ci/socket")
          # == and not <=: a lower peak would mean something else serialised the
          # fetches and this test proves nothing about the setting
          assert peak == ${toString expectedPeak}, (
              f"CI substitution peak was {peak}, expected exactly ${toString expectedPeak}"
          )

      with subtest("the system daemon is the control, and exceeds that cap"):
          # identical paths, cache, user and command -- only the socket differs
          peak = substitute_through("daemon")
          assert peak > ${toString expectedPeak}, (
              f"control peak was {peak}; the transport itself may be serialising, "
              "which would invalidate the measurement above"
          )
    '';
  }
