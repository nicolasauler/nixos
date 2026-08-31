# End-to-end rehearsal of the thing that actually broke: a nix-eval build that
# fans out to several nix-build children, on ONE worker, with the concurrency
# lock in force.
#
# Why this test exists. The deadlock that killed PR #12 was found by review, and
# is otherwise disproven only by unit assertions against a hand-built Worker
# object with stubbed `workerforbuilders` — the same shape of evidence that let
# the `Change.who` bug through. Here the real master schedules real builds:
#   * if the cap were `Worker.max_builds` again, the parked parent would hold the
#     only slot and its children could never start: this test would hang and fail
#   * if the MasterLock were attached to the eval or gcroot builders instead of
#     */nix-build, the parent could never reach its trigger step — same failure
#   * the lock's effect is measured, not assumed: live builds are sampled once a
#     second while the fan-out runs, and the peak is asserted
#   * and the deadlock is disproven directly, by observing a parked eval build
#     and one of its triggered children alive in the same sample
#
# GitHub is never contacted. buildbot-nix hardcodes a tokenised
# https://git:<token>@github.com/... clone URL for GitHub projects
# (github_projects.py:723), so a GitHub-backed project can never build offline;
# a pull-based repository uses its configured url verbatim, which is what makes
# this possible. Recipe adapted from buildbot-nix's own checks/poller.nix.
#
# Note buildPushes = true here: git-poller changes are push-category, so the
# desktop's `buildPushes = false` would filter them out and nothing would build.
# The author allowlist is therefore not what is under test — the lock and the
# fan-out are. checks/buildbot-workstation covers the policy wiring.
{
  pkgs,
  inputs,
  ...
}:
pkgs.testers.runNixOSTest {
  name = "buildbot-fanout";

  nodes.machine = {
    config,
    lib,
    pkgs,
    ...
  }: {
    imports = [
      inputs.certus-infra.inputs.buildbot-nix.nixosModules.buildbot-master
      inputs.certus-infra.inputs.buildbot-nix.nixosModules.buildbot-worker
      ../modules/services/buildbot-pr-policy.nix
    ];
    virtualisation = {
      memorySize = 6144;
      cores = 4;
      diskSize = 8192;
    };

    nix.settings.experimental-features = ["nix-command" "flakes"];
    networking.firewall.enable = false;

    services.buildbot-nix.master = {
      enable = true;
      domain = "localhost";
      authBackend = "none";
      admins = ["admin"];
      workersFile = pkgs.writeText "workers.json" ''
        [
          { "name": "local-worker", "pass": "test-password", "cores": 1 }
        ]
      '';
      pullBased = {
        pollInterval = 5;
        repositories.test-flake = {
          # a plain local path: git fetches it directly, no ssh setup needed
          url = "/srv/repos/test-flake.git";
          defaultBranch = "master";
        };
      };
    };

    services.buildbot-nix.worker = {
      enable = true;
      name = "local-worker";
      workers = 1;
      workerPasswordFile = pkgs.writeText "worker-password" "test-password";
    };

    # The policy under test. One nix build at a time, across all projects.
    services.workstationCiPolicy = {
      enable = true;
      prAuthors = ["nobody"];
      buildPushes = true;
      maxConcurrentNixBuilds = 1;
    };

    # the CI-scoped nix cap, as the desktop applies it
    systemd.services.buildbot-worker.environment.NIX_CONFIG = ''
      cores = 1
      max-jobs = 1
    '';

    environment.systemPackages = with pkgs; [curl jq git];

    # The fixture repo is created by root but cloned by the buildbot-worker
    # user, and git refuses to touch a repository owned by someone else
    # ("detected dubious ownership") — the clone exits 128 in ~30ms and the
    # eval build fails before it ever evaluates the flake.
    programs.git = {
      enable = true;
      config.safe.directory = "*";
    };

    # Samples the live-build list once a second so the lock's effect can be
    # measured after the fan-out finishes.
    environment.etc."fanout-probe.sh".source = pkgs.writeShellScript "fanout-probe" ''
      while true; do
        ${pkgs.curl}/bin/curl -sf localhost:8010/api/v2/builds?complete=false \
          | ${pkgs.jq}/bin/jq -c '[.builds[].builderid]' >> /tmp/live.jsonl 2>/dev/null || true
        ${pkgs.coreutils}/bin/sleep 1
      done
    '';

    systemd.services.setup-git-repo = {
      description = "Seed a local flake with a fan-out of slow checks";
      wantedBy = ["multi-user.target"];
      before = ["buildbot-master.service"];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "root";
        Environment = "HOME=/root";
      };
      path = with pkgs; [git coreutils nix];
      script = ''
        rm -rf /srv/repos/test-flake.git /tmp/test-flake
        mkdir -p /srv/repos
        git init --bare -b master /srv/repos/test-flake.git

        mkdir -p /tmp/test-flake
        cd /tmp/test-flake
        git init -b master
        git config user.name 'Test User'
        git config user.email 'test@example.com'

        system=$(nix config show system)
        # Three checks that each take long enough to overlap if the lock let
        # them. No nixpkgs dependency: /bin/sh always exists in the sandbox.
        cat > flake.nix << EOF
        {
          outputs = { self }: {
            checks.$system = {
              slow-one = derivation {
                name = "slow-one";
                system = "$system";
                builder = "/bin/sh";
                args = [ "-c" "sleep 8; echo one > \$out" ];
              };
              slow-two = derivation {
                name = "slow-two";
                system = "$system";
                builder = "/bin/sh";
                args = [ "-c" "sleep 8; echo two > \$out" ];
              };
              slow-three = derivation {
                name = "slow-three";
                system = "$system";
                builder = "/bin/sh";
                args = [ "-c" "sleep 8; echo three > \$out" ];
              };
            };
          };
        }
        EOF

        git add flake.nix
        git commit -m 'fan-out fixture'
        git remote add origin /srv/repos/test-flake.git
        git push -u origin master

        # the eval step clones this as the buildbot-worker user
        chmod -R a+rX /srv/repos
      '';
    };
  };

  testScript = ''
    import json

    machine.wait_for_unit("setup-git-repo.service")
    machine.wait_for_unit("buildbot-master.service")
    machine.wait_for_open_port(8010)
    machine.wait_for_unit("buildbot-worker.service")

    def api(path):
        return json.loads(machine.succeed(f"curl -sf 'localhost:8010/api/v2/{path}'"))

    # sample live builds for the whole run, so concurrency can be measured
    machine.succeed("systemd-run --unit=fanout-probe --collect /etc/fanout-probe.sh")

    with subtest("the pull-based project registers and the poller sees the commit"):
        machine.wait_until_succeeds(
            "curl -sf localhost:8010/api/v2/projects | grep -q test-flake", timeout=180
        )
        machine.wait_until_succeeds(
            "curl -sf localhost:8010/api/v2/builders | grep -q nix-build", timeout=180
        )

    names = {b["builderid"]: b["name"] for b in api("builders")["builders"]}
    assert any(n.endswith("/nix-build") for n in names.values()), names
    assert any(n.endswith("/nix-eval") for n in names.values()), names

    with subtest("a new commit reaches the poller"):
        # A git poller records the head it first sees without building it, so the
        # fixture commit alone triggers nothing — push a second one, as
        # buildbot-nix's own checks/poller.nix does.
        machine.succeed(
            "cd /tmp/test-flake && env HOME=/root git commit -q --allow-empty "
            "-m 'trigger the fan-out' && env HOME=/root git push -q origin master"
        )

    # THE ASSERTION THIS TEST EXISTS FOR: the fan-out must complete on one
    # worker. A per-worker cap, or a lock on the wrong builder, hangs here.
    with subtest("the fan-out completes on a single worker (no deadlock)"):
        # wait for the work to APPEAR and then finish; waiting only for an empty
        # live list passes instantly before anything has started
        machine.wait_until_succeeds(
            "curl -sf 'localhost:8010/api/v2/builds?complete=true&limit=100' "
            "| jq -e '[.builds[]] | length >= 4'",
            timeout=900,
        )
        machine.wait_until_succeeds(
            "curl -sf 'localhost:8010/api/v2/builds?complete=false' | jq -e '.builds | length == 0'",
            timeout=300,
        )

        # Re-read the builder list: the fanned-out builds are recorded against
        # VIRTUAL builders that only exist once the trigger runs. buildbot-nix
        # sets virtual_builder_name = "<ref>:<project>#<attr_prefix>.<attr>"
        # (build_trigger.py:199), so the request is scheduled on the real
        # */nix-build builder — which is where the lock lives — while the build
        # itself appears under the per-attribute name. Classifying on the real
        # builder alone finds nothing.
        names = {b["builderid"]: b["name"] for b in api("builders")["builders"]}
        attr_ids = {i for i, n in names.items() if "#" in n}
        eval_ids = {i for i, n in names.items() if n.endswith("/nix-eval")}

        builds = api("builds?complete=true&limit=100")["builds"]
        by_name = {}
        for b in builds:
            by_name.setdefault(names.get(b["builderid"], "?"), []).append(b["results"])

        fanned = [r for n, rs in by_name.items() if "#" in n for r in rs]
        nix_eval = [r for n, rs in by_name.items() if n.endswith("/nix-eval") for r in rs]

        assert len(fanned) >= 3, f"expected 3 fanned-out builds, got {by_name}"
        # 0 = SUCCESS, 1 = WARNINGS, 2 = FAILURE, 4 = RETRY
        assert all(r == 0 for r in fanned), f"a fanned-out build did not succeed: {by_name}"
        # The eval build is allowed to WARN: its "Evaluate scheduled effects"
        # step shells out to `buildbot-effects list-schedules`, which exits 1 on
        # a flake with no herculesCI output. That step is warnOnFailure, and the
        # fixture deliberately stays minimal rather than growing an effects
        # output just to keep it quiet. A real failure (2) still fails the test.
        assert nix_eval and all(r in (0, 1) for r in nix_eval), f"eval did not pass: {by_name}"

    machine.succeed("systemctl stop fanout-probe.service || true")

    samples = [
        json.loads(line)
        for line in machine.succeed("cat /tmp/live.jsonl").splitlines()
        if line.strip()
    ]

    with subtest("the lock actually bounded concurrent nix builds"):
        peaks = [len([i for i in s if i in attr_ids]) for s in samples]
        peak = max(peaks, default=0)
        assert peak >= 1, f"never observed a running fanned-out build in {len(samples)} samples"
        assert peak <= 1, (
            f"maxConcurrentNixBuilds=1 exceeded: observed {peak} concurrent "
            "fanned-out builds, so the master lock on */nix-build does not reach "
            "the virtual builders the children actually run under"
        )

    with subtest("the parked eval and its triggered child were alive together"):
        # the direct disproof of the max_builds deadlock: with ONE worker, the
        # parent parks in BuildTrigger while a child it triggered runs
        coexisted = any(
            any(i in eval_ids for i in s) and any(i in attr_ids for i in s)
            for s in samples
        )
        assert coexisted, (
            "never saw a parked eval build and a triggered child in the same "
            f"sample across {len(samples)} samples; the fan-out may have "
            "serialised for the wrong reason"
        )
  '';
}
