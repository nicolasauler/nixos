# End-to-end proof that the master lock, rather than Buildbot's per-pair worker
# constraint, serialises fanned-out nix build commands. Two workers are required:
# with one worker, the real nix-build builder has one WorkerForBuilder and peak 1
# holds even without a lock. The locked node is compared with an unlocked control.
# Active Build rows are not the metric either: simultaneous admission can start a
# second build that waits in locks_acquire while consuming its builder/worker pair.
{
  pkgs,
  inputs,
  ...
}: let
  node = maxConcurrentNixBuilds: {
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
          { "name": "local-worker", "pass": "test-password", "cores": 2 }
        ]
      '';
      pullBased = {
        pollInterval = 5;
        repositories.test-flake = {
          url = "/srv/repos/test-flake.git";
          defaultBranch = "master";
        };
      };
    };

    services.buildbot-nix.worker = {
      enable = true;
      name = "local-worker";
      workers = 2;
      workerPasswordFile = pkgs.writeText "worker-password" "test-password";
    };

    services.workstationCiPolicy = {
      enable = true;
      prAuthors = ["nobody"];
      buildPushes = true;
      inherit maxConcurrentNixBuilds;
    };

    systemd.services.buildbot-worker.environment.NIX_CONFIG = ''
      cores = 1
      max-jobs = 1
    '';

    environment.systemPackages = with pkgs; [curl jq git];
    programs.git = {
      enable = true;
      config.safe.directory = "*";
    };

    environment.etc."fanout-probe.sh".source = pkgs.writeShellScript "fanout-probe" ''
      while true; do
        live=$(
          ${pkgs.curl}/bin/curl -sf localhost:8010/api/v2/builds?complete=false \
            | ${pkgs.jq}/bin/jq -c '[.builds[] | {buildid, builderid}]'
        ) || live='[]'
        commands=0
        while read -r buildid; do
          if ${pkgs.curl}/bin/curl -sf \
            "localhost:8010/api/v2/builds/$buildid/steps" \
            | ${pkgs.jq}/bin/jq -e \
              'any(.steps[]; .name == "Build flake attr" and .started_at != null and .complete == false)' \
              >/dev/null; then
            commands=$((commands + 1))
          fi
        done < <(printf '%s' "$live" | ${pkgs.jq}/bin/jq -r '.[].buildid')
        ${pkgs.jq}/bin/jq -nc \
          --argjson builds "$live" \
          --argjson commands "$commands" \
          '{builds: $builds, commands: $commands}' >> /tmp/live.jsonl
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
        chmod -R a+rX /srv/repos
      '';
    };
  };
in
  pkgs.testers.runNixOSTest {
    name = "buildbot-fanout";
    nodes = {
      locked = node 1;
      unlocked = node null;
    };

    testScript = ''
      import json

      def api(machine, path):
          return json.loads(machine.succeed(f"curl -sf 'localhost:8010/api/v2/{path}'"))

      def run_fanout(machine):
          machine.wait_for_unit("setup-git-repo.service")
          machine.wait_for_unit("buildbot-master.service")
          machine.wait_for_open_port(8010)
          machine.wait_for_unit("buildbot-worker.service")

          machine.wait_until_succeeds(
              "curl -sf localhost:8010/api/v2/projects | grep -q test-flake", timeout=180
          )
          machine.wait_until_succeeds(
              "curl -sf localhost:8010/api/v2/builders | grep -q nix-build", timeout=180
          )
          machine.wait_until_succeeds(
              "curl -sf localhost:8010/api/v2/workers | grep -q local-worker-001", timeout=180
          )

          names = {b["builderid"]: b["name"] for b in api(machine, "builders")["builders"]}
          assert any(name.endswith("/nix-build") for name in names.values()), names
          assert any(name.endswith("/nix-eval") for name in names.values()), names

          machine.succeed(
              "systemd-run --unit=fanout-probe --collect /etc/fanout-probe.sh"
          )
          machine.succeed(
              "cd /tmp/test-flake && env HOME=/root git commit -q --allow-empty "
              "-m 'trigger the fan-out' && env HOME=/root git push -q origin master"
          )

          machine.wait_until_succeeds(
              "curl -sf 'localhost:8010/api/v2/builds?complete=true&limit=100' "
              "| jq -e '[.builds[]] | length >= 4'",
              timeout=900,
          )
          machine.wait_until_succeeds(
              "curl -sf 'localhost:8010/api/v2/builds?complete=false' "
              "| jq -e '.builds | length == 0'",
              timeout=300,
          )
          machine.succeed("systemctl stop fanout-probe.service || true")

          names = {b["builderid"]: b["name"] for b in api(machine, "builders")["builders"]}
          attr_ids = {builder_id for builder_id, name in names.items() if "#" in name}
          eval_ids = {
              builder_id
              for builder_id, name in names.items()
              if name.endswith("/nix-eval")
          }

          builds = api(machine, "builds?complete=true&limit=100")["builds"]
          by_name = {}
          for build in builds:
              by_name.setdefault(names.get(build["builderid"], "?"), []).append(
                  build["results"]
              )

          fanned = [
              result
              for name, results in by_name.items()
              if "#" in name
              for result in results
          ]
          nix_eval = [
              result
              for name, results in by_name.items()
              if name.endswith("/nix-eval")
              for result in results
          ]
          assert len(fanned) >= 3, f"expected 3 fanned-out builds, got {by_name}"
          assert all(result == 0 for result in fanned), by_name
          assert nix_eval and all(result in (0, 1) for result in nix_eval), by_name

          samples = [
              json.loads(line)
              for line in machine.succeed("cat /tmp/live.jsonl").splitlines()
              if line.strip()
          ]
          active_peaks = [
              len(
                  [
                      build
                      for build in sample["builds"]
                      if build["builderid"] in attr_ids
                  ]
              )
              for sample in samples
          ]
          active_peak = max(active_peaks, default=0)
          command_peak = max((sample["commands"] for sample in samples), default=0)
          assert active_peak >= 1, (
              f"never observed a fanned-out build in {len(samples)} samples"
          )
          assert command_peak >= 1, (
              f"never observed a running Build flake attr step in {len(samples)} samples"
          )

          coexisted = any(
              any(build["builderid"] in eval_ids for build in sample["builds"])
              and any(build["builderid"] in attr_ids for build in sample["builds"])
              for sample in samples
          )
          assert coexisted, "never observed the parked eval beside a triggered child"
          return active_peak, command_peak

      with subtest("two workers remain serialised by maxConcurrentNixBuilds=1"):
          locked_active, locked_commands = run_fanout(locked)
          assert locked_commands == 1, (
              f"counting lock admitted {locked_commands} running build commands"
          )
          assert locked_active >= 2, (
              "the locked case never exposed an active lock waiter; it does not "
              "demonstrate why counting Build rows is the wrong metric"
          )
      locked.shutdown()

      with subtest("without the lock the same two workers overlap build commands"):
          _, unlocked_commands = run_fanout(unlocked)
          assert unlocked_commands >= 2, (
              f"control peaked at {unlocked_commands}; the experiment does not "
              "distinguish the master lock from Buildbot's per-pair constraint"
          )
    '';
  }
