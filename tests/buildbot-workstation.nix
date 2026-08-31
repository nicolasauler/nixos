# VM rehearsal for the buildbot workstation policy.
#
# Why this exists: three bugs in a row reached the real desktop because every
# check was static (nix eval / unit text) or used a hand-written stub:
#   * #9's author filter read `change.author`; the real object exposes `who`,
#     so the filter raised AttributeError and silently dropped every PR. The
#     unit test passed because it asserted against SimpleNamespace(author=...).
#   * #6's "4 slots" capped worker processes, not builds.
#   * #12's first draft capped nix-daemon globally, which would have degraded
#     the user's own builds.
# A booted VM catches the first class outright: if the configurator raises, the
# master does not start, and the test fails here instead of on the workstation.
#
# Deliberately NOT covered (needs GitHub, so it belongs in a later iteration):
# project enumeration, the real webhook path, and end-to-end fan-out.
{
  pkgs,
  inputs,
  ...
}:
pkgs.testers.runNixOSTest {
  name = "buildbot-workstation";

  nodes.machine = {
    config,
    lib,
    pkgs,
    ...
  }: {
    imports = [
      inputs.certus-infra.nixosModules.buildbot
      ../modules/services/buildbot-limits.nix
      ../modules/services/nix-daemon-ci.nix
      ../modules/services/buildbot-pr-policy.nix
    ];

    virtualisation = {
      memorySize = 4096;
      cores = 2;
      diskSize = 8192;
    };

    # Host-specific bits that cannot work in a VM.
    services.tailscale.enable = lib.mkForce false;
    systemd.services.tailscale-funnel-buildbot.enable = false;
    networking.firewall.enable = lib.mkForce false;

    # the desktop enables these; the node needs them for `nix config show`
    nix.settings.experimental-features = ["nix-command" "flakes"];

    # The certus module points at two secrets outside the store. Their contents
    # only matter for talking to GitHub, which this test never does — but the
    # App key must parse as RSA or the master aborts while minting its JWT.
    # These must exist before the unit starts: systemd resolves LoadCredential=
    # before ExecStartPre, so a preStart hook is too late (fails 243/CREDENTIALS).
    system.activationScripts.buildbotTestSecrets = {
      deps = ["users"];
      text = ''
        mkdir -p /var/lib/buildbot
        ${pkgs.openssl}/bin/openssl genrsa -out /var/lib/buildbot/github-app.key 2048 2>/dev/null
        printf 'dummy-oauth-secret' > /var/lib/buildbot/oauth-secret
        chown -R buildbot:buildbot /var/lib/buildbot
        chmod 0600 /var/lib/buildbot/github-app.key /var/lib/buildbot/oauth-secret
      '';
    };
  };

  testScript = ''
    machine.wait_for_unit("postgresql.service")

    # If WorkstationPolicyConfigurator raises, master.cfg fails to load and the
    # unit never comes up. This is the assertion the workstation had to make.
    machine.wait_for_unit("buildbot-master.service")
    machine.wait_for_open_port(8010)
    machine.wait_for_unit("buildbot-worker.service")

    with subtest("CI build parallelism is capped on the worker, not globally"):
        env = machine.succeed("systemctl show buildbot-worker.service -p Environment --value")
        assert "cores = 4" in env, f"worker NIX_CONFIG missing cores: {env}"
        assert "max-jobs = 1" in env, f"worker NIX_CONFIG missing max-jobs: {env}"

    with subtest("NIX_CONFIG actually binds cores/max-jobs on this nix"):
        # proves the mechanism the cap relies on, in situ
        cores = machine.succeed("NIX_CONFIG='cores = 4' nix config show cores").strip()
        assert cores == "4", f"NIX_CONFIG did not bind cores: {cores}"
        jobs = machine.succeed("NIX_CONFIG='max-jobs = 1' nix config show max-jobs").strip()
        assert jobs == "1", f"NIX_CONFIG did not bind max-jobs: {jobs}"

    with subtest("an untrusted client's cores REACHES THE BUILDER via the shared daemon"):
        # The whole premise of the CI-scoped cap: compilers run under
        # nix-daemon, and the daemon applies a client's buildCores even when
        # that client is untrusted (daemon.cc ClientSettings::apply assigns it
        # before the `trusted ||` gate). Probe it for real: a derivation that
        # echoes $NIX_BUILD_CORES, built once uncapped and once as the
        # buildbot-worker user carrying the cap. Distinct names, because
        # NIX_BUILD_CORES is not part of the drv hash and a same-name build
        # would just return the cached result.
        def probe(name):
            return (
                'derivation { name = "' + name + '"; system = "x86_64-linux"; '
                'builder = "/bin/sh"; args = [ "-c" "echo $NIX_BUILD_CORES > $out" ]; }'
            )

        out = machine.succeed(
            "nix build --no-link --print-out-paths --expr '" + probe("cores-probe-uncapped") + "'"
        ).strip()
        uncapped = machine.succeed("cat " + out).strip()

        out = machine.succeed(
            "runuser -u buildbot-worker -- env HOME=/tmp NIX_CONFIG='cores = 4' "
            "nix build --no-link --print-out-paths --expr '" + probe("cores-probe-ci") + "'"
        ).strip()
        capped = machine.succeed("cat " + out).strip()

        # the VM has 2 vCPUs and global cores = 0, so an uncapped build sees 2
        assert uncapped == "2", f"uncapped build saw {uncapped} cores, expected the VM's 2"
        assert capped == "4", f"CI cap did not reach the builder: saw {capped}, expected 4"

    with subtest("global nix is left alone"):
        # the user's own builds must not inherit the CI cap
        cores = machine.succeed("nix config show cores").strip()
        assert cores == "0", f"global cores was capped: {cores}"
        jobs = machine.succeed("nix config show max-jobs").strip()
        assert jobs != "1", f"global max-jobs was capped: {jobs}"

    with subtest("nix-daemon carries no limits of ours"):
        high = machine.succeed("systemctl show nix-daemon.service -p MemoryHigh --value").strip()
        assert high == "infinity", f"MemoryHigh set on nix-daemon: {high}"
        nice = machine.succeed("systemctl show nix-daemon.service -p Nice --value").strip()
        assert nice == "0", f"Nice set on nix-daemon: {nice}"

    with subtest("the CI daemon carries the limits instead, and only it"):
        machine.wait_for_unit("nix-daemon-ci.socket")
        for prop, expected in [
            # systemd normalises sizes to bytes
            ("MemoryHigh", str(12 * 1024**3)),
            ("Nice", "19"),
        ]:
            got = machine.succeed(
                f"systemctl show nix-daemon-ci.service -p {prop} --value"
            ).strip()
            assert got == expected, f"CI daemon {prop}={got}, expected {expected}"
        # the setting the untrusted client provably cannot set for itself
        env = machine.succeed("systemctl show nix-daemon-ci.service -p Environment --value")
        assert "max-substitution-jobs = 4" in env, f"CI daemon env missing the cap: {env}"
        # and the socket is reachable only by CI's group
        mode = machine.succeed("stat -c '%a %G' /run/nix-daemon-ci/socket").strip()
        assert mode == "660 buildbot-worker", f"unexpected socket mode/group: {mode}"

    with subtest("CI builds run in the CI daemon's cgroup, not the system one"):
        # This is the whole point of the second daemon, so prove it at the
        # cgroup level rather than by inference. Note a daemon does NOT log the
        # builds it performs to its journal — daemon.cc tunnels build output to
        # the client — so the observable is cgroup membership: the daemon forks
        # the builder, so builder processes live in the daemon's subtree.
        # The CI daemon is socket-activated, so before the first connection its
        # cgroup does not exist at all — that is the cleanest possible baseline.
        def cg_procs(unit):
            out = machine.succeed(
                f"( find /sys/fs/cgroup/system.slice/{unit} -name cgroup.procs "
                "-exec cat {} + 2>/dev/null || true ) | wc -l"
            )
            return int(out.strip())

        base_ci = cg_procs("nix-daemon-ci.service")
        base_sys = cg_procs("nix-daemon.service")

        routing_probe = (
            'derivation { name = "ci-daemon-routing-probe"; system = "x86_64-linux"; '
            'builder = "/bin/sh"; args = [ "-c" "sleep 8; echo routed > $out" ]; }'
        )
        machine.succeed(
            "systemd-run --unit=ci-probe-build --collect --uid=buildbot-worker "
            "--setenv=HOME=/tmp --setenv=NIX_REMOTE=unix:///run/nix-daemon-ci/socket "
            "/run/current-system/sw/bin/nix build --no-link --extra-experimental-features "
            f"nix-command --expr '{routing_probe}'"
        )

        peak_ci, peak_sys = base_ci, base_sys
        for _ in range(30):
            peak_ci = max(peak_ci, cg_procs("nix-daemon-ci.service"))
            peak_sys = max(peak_sys, cg_procs("nix-daemon.service"))
            if peak_ci > base_ci:
                break
            machine.succeed("sleep 1")

        machine.wait_until_fails("systemctl is-active ci-probe-build.service", timeout=120)

        assert peak_ci > base_ci, (
            f"no processes appeared in the CI daemon's cgroup (base {base_ci}, "
            f"peak {peak_ci}) — the build was not served by it"
        )
        assert peak_sys == base_sys, (
            f"the system daemon's cgroup grew ({base_sys} -> {peak_sys}), so CI "
            "work is still landing in the daemon that serves interactive builds"
        )
        # and the build really did happen
        machine.succeed("journalctl -u ci-probe-build --no-pager | grep -qv 'error'")

    with subtest("the worker cgroup limits still apply to the eval step"):
        quota = machine.succeed("systemctl show buildbot-worker.service -p CPUQuotaPerSecUSec --value").strip()
        assert quota != "infinity", "expected certus-infra's CPUQuota on the worker"

    with subtest("exactly one worker slot exists"):
        machine.wait_until_succeeds(
            "curl -sf localhost:8010/api/v2/workers | grep -q desktop-000"
        )
        workers = machine.succeed("curl -sf localhost:8010/api/v2/workers")
        assert "desktop-001" not in workers, f"more than one slot: {workers}"

    with subtest("master is functional (its own schedulers exist)"):
        machine.wait_until_succeeds(
            "curl -sf localhost:8010/api/v2/schedulers | grep -q reload-github-projects"
        )
  '';
}
