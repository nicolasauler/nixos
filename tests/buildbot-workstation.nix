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

    # buildbot-limits.nix sets min-free = 20GiB, which exceeds this VM's whole
    # 8 GB disk, so nix runs a full auto-GC before every build ("running auto-GC
    # to free 62370164736 bytes") and deletes the outputs earlier subtests just
    # produced. Scope it down here; the value itself is a real question for the
    # desktop, not something this test should assert.
    nix.settings.min-free = lib.mkForce 0;
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
        # The single line that makes this whole module do anything: without it the
        # worker talks to the SYSTEM daemon and every cap below is decoration.
        # Two reviewers independently showed the suite went 10/10 green with this
        # broken — one by deleting it, one by mkForce-ing it back to the system
        # socket — because the routing subtest below supplies NIX_REMOTE itself.
        assert "NIX_REMOTE=unix:///run/nix-daemon-ci/socket" in env, (
            f"worker is not pointed at the CI daemon: {env}"
        )

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
        # `!= "1"` is weak on its own — a reviewer noted it also holds for a
        # regression that set global max-jobs to some other value. A literal
        # cannot be pinned instead, because this is the EFFECTIVE value and it is
        # environment-dependent: "2" in this 2-vCPU VM, "auto" on desktop, and
        # the test framework writes its own max-jobs into global nix.conf. So
        # assert the invariant that does not vary — the CI cap's exact values
        # must appear in the worker's environment and nowhere in global config.
        conf = machine.succeed("cat /etc/nix/nix.conf")
        assert "max-jobs = 1" not in conf, f"CI max-jobs cap leaked globally:\n{conf}"
        assert "cores = 4" not in conf, f"CI cores cap leaked globally:\n{conf}"

    # These two loops are deliberately the SAME property list. Every limit this
    # module introduces has to be present on the CI daemon and absent from the
    # system one; asserting only the two properties that predate the fix let a
    # reviewer delete MemoryMax and MemorySwapMax — the entire substance of the
    # memory fix — and still see 10/10 green, and let the #12 regression (limits
    # leaking onto the user's daemon) pass for the four newer properties.
    ci_limits = [
        # (property, expected on the CI daemon, expected on the SYSTEM daemon).
        # systemd normalises sizes to bytes and reports unset weights as
        # "[not set]". None means "shared with upstream, do not guard": nixpkgs'
        # own nix-daemon.service already sets OOMPolicy=continue, so asserting it
        # absent there fails for a reason that has nothing to do with this module
        # — found by running this test rather than reasoning about it.
        ("MemoryHigh", str(12 * 1024**3), "infinity"),
        ("MemoryMax", str(16 * 1024**3), "infinity"),
        ("MemorySwapMax", "0", "infinity"),
        ("CPUWeight", "20", "[not set]"),
        ("IOWeight", "20", "[not set]"),
        ("OOMPolicy", "continue", None),
        ("Nice", "19", "0"),
    ]

    with subtest("nix-daemon carries no limits of ours"):
        for prop, _ci, unset in ci_limits:
            if unset is None:
                continue
            got = machine.succeed(
                f"systemctl show nix-daemon.service -p {prop} --value"
            ).strip()
            assert got == unset, f"{prop} set on the system daemon: {got}"

    with subtest("the CI daemon carries the limits instead, and only it"):
        machine.wait_for_unit("nix-daemon-ci.socket")
        for prop, expected, _unset in ci_limits:
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
        # Ask the BUILDER which cgroup it is in, rather than counting processes.
        #
        # An earlier version of this test counted PIDs in each daemon's cgroup
        # subtree and was confounded: the CI daemon is socket-activated, so
        # merely connecting starts its main process and grows the count before
        # any builder runs. It also ended with `journalctl | grep -qv error`,
        # which passes as long as ANY line lacks the word — worthless. A
        # derivation that writes /proc/self/cgroup into its own output cannot be
        # fooled either way: the daemon forks the builder, so the builder's
        # cgroup IS the serving daemon's.
        # NB: only shell builtins. The nix sandbox provides /bin/sh and nothing
        # else — an earlier version used `cat` and died with exit 127, while the
        # other probes in this file work only because `echo` is a builtin.
        cgroup_probe = (
            'derivation { name = "ci-daemon-routing-probe"; system = "x86_64-linux"; '
            'builder = "/bin/sh"; args = [ "-c" '
            '"while IFS= read -r l; do echo $l; done < /proc/self/cgroup > $out" ]; }'
        )
        out = machine.succeed(
            "runuser -u buildbot-worker -- env HOME=/tmp "
            "NIX_REMOTE=unix:///run/nix-daemon-ci/socket "
            f"nix build --no-link --print-out-paths --expr '{cgroup_probe}'"
        ).strip()
        cgroup = machine.succeed(f"cat {out}").strip()

        assert "nix-daemon-ci.service" in cgroup, (
            f"the builder did not run under the CI daemon: {cgroup}"
        )
        assert "/nix-daemon.service" not in cgroup, (
            f"the builder ran under the system daemon: {cgroup}"
        )

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
