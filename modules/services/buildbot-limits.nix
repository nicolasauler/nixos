# Capacity limits for buildbot-nix now that a SHARED repo builds here.
#
# bipa-app/bipa has five checks per PR (my-crate, clippy, fmt, nextest, schema)
# and several PRs open at any time, so "whatever certus-infra sized for two
# personal repos" is no longer the right shape. The certus module already caps
# the worker at CPUQuota=75% / MemoryMax=24G with OOMPolicy=continue (a job
# dies, the worker survives) — these are the knobs it leaves wide open.
#
# Arithmetic, for a 16-thread box under a 75% quota (~12 threads of real CPU):
#   4 slots x cores=3 = 12 threads requested = the quota. max-jobs=2 lets a
#   slot's dependency graph parallelise a little without a slot ever behaving
#   like it owns the machine. Want 5 concurrent builds instead of 4? Change
#   both numbers below (workersFile `cores` and worker.workers must agree —
#   the master spawns one slot per `cores`, buildbot_nix/__init__.py:114).
{
  lib,
  pkgs,
  ...
}: {
  # One slot per `cores`. mkForce because certus-infra sets 8.
  # NOTE: the worker password is a plaintext literal in the store — inherited
  # from certus-infra's module (writeText), not introduced here. It has to
  # match worker.workerPasswordFile, so it is repeated rather than fixed;
  # fixing it belongs in that repo.
  services.buildbot-nix.master.workersFile = lib.mkForce (pkgs.writeText "workers.json" ''
    [
      { "name": "desktop", "pass": "certus-worker-local", "cores": 4 }
    ]
  '');
  services.buildbot-nix.worker.workers = lib.mkForce 4;

  # evalWorkerCount was null => one worker per core (~16), each allowed
  # evalMaxMemorySize (2048 MB) => up to ~32 GB of eval inside a 24 GB cgroup.
  # bipa's flake is large enough for that to be the first thing to OOM.
  services.buildbot-nix.master.evalWorkerCount = 2;

  nix.settings = {
    # cores = 0 means "all threads" per derivation: with several slots running,
    # every build would think it owns the machine.
    cores = 3;
    # Bounds fan-out *within* one slot's `nix build`. Interactive builds can
    # still opt out per invocation with `--max-jobs N`.
    max-jobs = 2;
    # CI churn makes an unbounded store a matter of time. min-free triggers a
    # GC mid-build when space runs low; max-free is how much to reclaim.
    min-free = 20 * 1024 * 1024 * 1024; # 20 GiB
    max-free = 60 * 1024 * 1024 * 1024; # 60 GiB
  };

  # Old generations, weekly. PR builds are not gcrooted (buildbot registers
  # roots per branch-glob, default branch only) and its tmpfiles rule ages out
  # gcroot drvs after 7d, so this mostly reclaims superseded system/CI paths.
  nix.gc = {
    automatic = true;
    dates = "03:15";
    options = "--delete-older-than 14d";
  };
}
