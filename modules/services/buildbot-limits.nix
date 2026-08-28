# Capacity limits for buildbot-nix — scoped to CI, never global.
#
# THE THING THAT KEPT BITING: `nix build` compiles nothing itself. It hands the
# derivation to nix-daemon over a socket, so every compiler process lives in
# nix-daemon.service's cgroup, NOT buildbot-worker's. certus-infra's
# CPUQuota=75% / MemoryMax=24G on the worker therefore bound only the *eval*
# step (nix-eval-jobs, which buildbot does spawn itself), and nixpkgs ships
# nix-daemon with no CPU/memory/IO limits at all. CI compilation was effectively
# unbounded no matter what buildbot's slot count said — which is why this box
# was unusable at 8 concurrent builds and still stuttered at 2.
#
# The tempting fix is to cap nix-daemon. Rejected: that unit serves every
# `nix build` on this host, so it would degrade interactive work too (MemoryHigh
# especially, which reclaim-throttles a big local build even while CI is idle).
#
# What is used instead: cap CI *as a client*. See the NIX_CONFIG block below —
# the daemon honours a client's cores/max-jobs even from an untrusted user, so
# the cap lands on CI alone and global nix.settings stay free for you.
#
# Aggravating factors on this host, worth knowing: no swap at all (memory
# pressure goes to reclaim stalls / OOM, never to swap), and min-free=20GiB means
# a build can trigger a store GC mid-compile if the disk sits near that mark.
#
# Concurrency is one build at a time, enforced in three independent places:
#   * buildbot slots — workersFile `cores` + worker.workers, which must agree
#     (buildbot_nix/__init__.py:114)
#   * max_builds_per_worker in buildbot-pr-policy.nix — slots alone do NOT bound
#     builds, since buildbot-nix leaves max_builds unset (= unlimited)
#   * max-jobs in the worker's NIX_CONFIG, which bounds what nix itself will run
# Want CI faster? Raise all three together, and expect the desktop to feel it.
# The next lever, if CPU/IO priority still bites: a second nix-daemon instance
# with its own cgroup limits on a private socket (NIX_REMOTE=unix://…), sharing
# this store — real isolation without degrading your daemon. Not done yet.
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
      { "name": "desktop", "pass": "certus-worker-local", "cores": 1 }
    ]
  '');
  services.buildbot-nix.worker.workers = lib.mkForce 1;

  # evalWorkerCount was null => one worker per core (~16), each allowed
  # evalMaxMemorySize (2048 MB) => up to ~32 GB of eval inside a 24 GB cgroup.
  # bipa's flake is large enough for that to be the first thing to OOM.
  services.buildbot-nix.master.evalWorkerCount = 2;

  # CI's build parallelism, scoped to the worker ONLY — this is the knob that
  # bounds compilation without touching your own nix at all.
  #
  # `nix build` hands the derivation to the shared nix-daemon, so the worker's
  # cgroup limits never reach the compilers. But the daemon applies the
  # *client's* cores/max-jobs unconditionally: in nix 2.34.8
  # src/libstore/daemon.cc, ClientSettings::apply assigns buildCores and
  # maxBuildJobs *before* the `trusted ||` gate that restricts everything else,
  # so an untrusted client (buildbot-worker is not in trusted-users) still gets
  # these honoured. The nix client picks them up from NIX_CONFIG. Verified:
  # `NIX_CONFIG="cores = 7" nix config show cores` -> 7.
  #
  # Consequence: global nix.settings stay at whatever suits interactive use,
  # and only CI is capped. Deliberately NOT setting nix.settings.cores /
  # max-jobs, and deliberately NOT putting Nice/CPUWeight/IOWeight/MemoryHigh
  # on nix-daemon: that unit serves every `nix build` on this host, including
  # yours, and MemoryHigh in particular would reclaim-throttle your own builds
  # even while CI is idle.
  systemd.services.buildbot-worker.environment.NIX_CONFIG = ''
    cores = 4
    max-jobs = 1
  '';

  nix.settings = {
    # CI churn makes an unbounded store a matter of time. min-free triggers a
    # GC mid-build when space runs low; max-free is how much to reclaim. Global
    # on purpose — this is disk hygiene, not a performance cap.
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
