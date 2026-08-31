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
# Aggravating factor worth knowing: min-free=20GiB means a build can trigger a
# store GC mid-compile if the disk sits near that mark. (There IS a swap
# partition on this host — hosts/desktop/hardware-configuration.nix:31 — so
# memory pressure degrades to swap thrash before it reaches the OOM killer.)
#
# Concurrency is one compiling build at a time, from two knobs that bound
# different things:
#   * max_concurrent_nix_builds in buildbot-pr-policy.nix — a buildbot master
#     lock on every */nix-build builder, so at most one nix build runs across
#     all projects. This is NOT the worker slot count: buildbot already runs at
#     most one build per (builder, worker) pair (process/builder.py:319), and
#     capping Worker.max_builds instead DEADLOCKS buildbot-nix — the nix-eval
#     build parks in its BuildTrigger step holding a slot until the nix-build
#     builds it triggered finish (buildbot_nix/build_trigger.py:771), and those
#     need a slot on the same worker.
#   * max-jobs in the worker's NIX_CONFIG, which bounds derivations *within*
#     one nix build.
# workersFile `cores` and worker.workers still must agree (the master spawns one
# slot per `cores`, buildbot_nix/__init__.py:114); they set how many builds may
# be in flight at all, not how many compile.
# Want CI faster? Raise max_concurrent_nix_builds and cores together, and expect
# the desktop to feel it. The next lever, if CPU/IO priority still bites: a
# second nix-daemon instance with its own cgroup limits on a private socket
# (NIX_REMOTE=unix://… + NIX_DAEMON_SOCKET_PATH on the service), sharing this
# store — real isolation without degrading your daemon. Not done yet.
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
  # Slots are not the compile cap (see the header). One is enough: buildbot
  # gives each (builder, worker) pair its own slot, so this single worker still
  # runs the parked nix-eval build and the nix-build it triggered side by side.
  services.buildbot-nix.worker.workers = lib.mkForce 1;

  # evalWorkerCount was null => one worker per core (~16), each allowed
  # evalMaxMemorySize (2048 MB) => up to ~32 GB of eval inside a 24 GB cgroup.
  # bipa's flake is large enough for that to be the first thing to OOM.
  services.buildbot-nix.master.evalWorkerCount = 2;

  # This host's policy. The module itself (buildbot-pr-policy.nix) only declares
  # the options, so the fan-out VM test can exercise it with buildPushes = true
  # (pull-based changes are push-category and would otherwise be filtered out).
  services.workstationCiPolicy = {
    enable = true;
    prAuthors = ["nicolasauler"];
    buildPushes = false;
    maxConcurrentNixBuilds = 1;
  };

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
  #
  # Substitution is now bounded too, but NOT here: the daemon runs the
  # substitution goals for a daemon-store client, and max-substitution-jobs
  # (default 16, nix worker-settings.hh:73-76) is not one of the two fields the
  # daemon takes unconditionally — it rides the `overrides` map and dies at the
  # `trusted ||` gate (daemon.cc:296-308), so this untrusted worker cannot lower
  # it. Adding buildbot-worker to trusted-users would also let it set
  # substituters and skip signature checks, and lowering it globally would slow
  # your own fetches. So CI gets its own daemon instead: see
  # nix-daemon-ci.nix, which carries `max-substitution-jobs` in the daemon's own
  # environment (nix reads NIX_CONFIG in any nix process, globals.cc:147-151)
  # alongside real cgroup limits.
  services.nixDaemonCi.enable = true;

  systemd.services.buildbot-worker.environment = {
    NIX_CONFIG = ''
      cores = 4
      max-jobs = 1
    '';
    # every CI `nix build` is served by the CI daemon, so its compilers land in
    # that unit's cgroup instead of the one serving your interactive builds
    NIX_REMOTE = "unix:///run/nix-daemon-ci/socket";
  };

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
