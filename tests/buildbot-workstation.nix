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
