# Restrict what this workstation is willing to build, and how much of it at once.
#
# THE PROBLEM. buildbot-nix builds every pull request on every registered repo
# ("build all pull requests" in project_config.py — no author filter, and no
# NixOS option for one), and additionally builds every push to the default
# branch (models.py `do_run()` returns true for it unconditionally, whatever
# `master.branches` says). For a shared repo like bipa-app/bipa that is a
# firehose: one event fans out to five heavy checks. Commissioning this box as a
# PR gate for a whole team took the machine down twice.
#
# THE MECHANISM. buildbot exposes `services.buildbot-master.configurators`, and
# buildbot runs them in list order before `load_schedulers`
# (buildbot/config/master.py:345-347), so an appended configurator can
# post-process the scheduler, builder and worker lists that buildbot-nix just
# built — no patching of upstream needed.
#
# WHY NOT THE RUNTIME TOGGLE. buildbot stores an `enabled` flag per scheduler in
# postgres, so `-prs` can be disabled through the UI/API instead. That state is
# invisible from this repo, is not reproduced by a rebuild, and a reconfig that
# recreates schedulers can quietly restore it. This is the declarative
# equivalent, and adding a collaborator later is a one-word change.
#
# The values live with each host (see buildbot-limits.nix for this desktop) so
# that tests can exercise the same module with different policy — notably
# `buildPushes = true`, which the fan-out VM test needs because pull-based
# (git-poller) changes are push-category, not pull.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.workstationCiPolicy;

  python = config.services.buildbot-nix.packages.python;
  policyPackage = python.pkgs.toPythonModule (pkgs.runCommand "buildbot-pr-policy" {
    nativeBuildInputs = [
      python
      (python.pkgs.toPythonModule config.services.buildbot-nix.packages.buildbot)
    ];
  } ''
    cp ${./buildbot-pr-policy.py} buildbot_pr_policy.py
    PYTHONPATH="$PWD''${PYTHONPATH:+:$PYTHONPATH}" \
      python ${./buildbot-pr-policy-test.py}
    install -Dm0444 buildbot_pr_policy.py \
      "$out/${python.sitePackages}/buildbot_pr_policy.py"
  '');

  pyBool = b:
    if b
    then "True"
    else "False";
  pyOptInt = n:
    if n == null
    then "None"
    else toString n;
in {
  options.services.workstationCiPolicy = {
    enable = lib.mkEnableOption "the workstation CI policy configurator";

    prAuthors = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      example = ["nicolasauler"];
      description = ''
        Forge logins allowed to trigger pull-request builds. Pull-request
        changes carry the bare login as the change author, unlike pushes which
        carry a git identity, so these are matched case-insensitively against
        `Change.who`. Must be non-empty: an empty policy would admit nothing
        and is rejected rather than silently blocking every build.
      '';
    };

    buildPushes = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Whether to build pushes at all — the default branch, `master.branches`
        globs, and the merge queue. False removes those schedulers outright.
        Note that pull-based (git-poller) repositories deliver push-category
        changes, so they build only when this is true.
      '';
    };

    maxConcurrentNixBuilds = lib.mkOption {
      type = lib.types.nullOr (lib.types.addCheck lib.types.int (x: x > 0));
      default = 1;
      description = ''
        How many `*/nix-build` builds may run at once across every project,
        enforced with a counting buildbot master lock. Null leaves builds
        unbounded, as upstream does.

        This is deliberately NOT `Worker.max_builds`: a `nix-eval` build parks
        in its BuildTrigger step holding a worker slot until the `nix-build`
        builds it triggered finish, so capping slots deadlocks the fan-out.
        The master lock bounds the heavy build steps. Under simultaneous
        dispatch a lock waiter may still occupy one `(builder, worker)` pair,
        so worker-wide slots must remain uncapped.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.prAuthors != [];
        message = "services.workstationCiPolicy.prAuthors must not be empty (the configurator rejects an empty allowlist).";
      }
    ];

    services.buildbot-master = {
      pythonPackages = _: [policyPackage];
      extraImports = lib.mkAfter ''
        from buildbot_pr_policy import WorkstationPolicyConfigurator
      '';
      configurators = lib.mkAfter [
        ''
          WorkstationPolicyConfigurator(
            # JSON arrays are valid Python and escape quotes/backslashes.
            pr_authors=${builtins.toJSON cfg.prAuthors},
            build_pushes=${pyBool cfg.buildPushes},
            max_concurrent_nix_builds=${pyOptInt cfg.maxConcurrentNixBuilds},
          )
        ''
      ];
    };
  };
}
