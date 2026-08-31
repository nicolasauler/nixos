# A second nix-daemon, dedicated to CI, sharing this machine's store.
#
# WHY THIS EXISTS. `nix build` compiles nothing itself: it hands the derivation
# to a nix-daemon over a socket, so every compiler process lives in that
# daemon's cgroup. That is why capping `buildbot-worker` never bounded CI
# compilation — the worker cgroup only ever held the eval step. The obvious fix,
# capping `nix-daemon.service`, was rejected because that unit serves every
# `nix build` on this host, including interactive ones: `MemoryHigh` there
# reclaim-throttles a local build even while CI is idle.
#
# So CI gets its own daemon instance on a private socket. Same store, so nothing
# is duplicated and everything CI builds is immediately available to you; a
# separate cgroup, so its limits are real and yours are untouched.
#
# WHAT THIS BOUNDS THAT `NIX_CONFIG` ON THE WORKER CANNOT. The client-side cap
# in buildbot-limits.nix works because nix's daemon applies a client's
# `cores`/`max-jobs` unconditionally (src/libstore/daemon.cc,
# ClientSettings::apply, before the `trusted ||` gate). Everything else travels
# in the trust-gated `overrides` map (daemon.cc:296-308), and `buildbot-worker`
# is deliberately not in `trusted-users` — granting it trust would also let it
# add substituters and skip signature checks. The consequence is that CI could
# not lower `max-substitution-jobs`, which defaults to 16: sixteen concurrent
# NAR fetch-and-decompress jobs, unbounded, in the shared daemon's cgroup. That
# is the same class of unbounded demand the compile caps closed, and it is a
# prime suspect for the IO stalls that made this machine unusable.
#
# A daemon reads NIX_CONFIG like any other nix process (globals.cc:147-151), so
# this instance simply carries the setting in its own environment — no trust
# grant, and no change to your defaults.
#
# TWO DAEMONS, ONE STORE is safe by nix's own design: the store is protected by
# per-path lock files and `/nix/var/nix/temproots`, which is exactly the
# mechanism that lets arbitrarily many `nix` processes and daemon forks share it.
# This adds another daemon process, not another store.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.nixDaemonCi;
  socketDir = "/run/nix-daemon-ci";
in {
  options.services.nixDaemonCi = {
    enable = lib.mkEnableOption "a second, cgroup-limited nix-daemon for CI";

    group = lib.mkOption {
      type = lib.types.str;
      default = "buildbot-worker";
      description = ''
        Group allowed to connect to the CI daemon socket. Only CI should reach
        it; everything else keeps using the system daemon.
      '';
    };

    nixConfig = lib.mkOption {
      type = lib.types.lines;
      default = ''
        max-substitution-jobs = 4
        min-free = 0
        max-free = 0
      '';
      description = ''
        NIX_CONFIG for the CI daemon itself. This is where settings the
        untrusted client cannot set belong — `max-substitution-jobs` above all,
        which the trust gate keeps out of a client's reach. Verified at the
        mechanism level, not just as a string in a unit: a daemon started with
        `NIX_CONFIG='post-build-hook = …'` runs that hook daemon-side.

        `min-free`/`max-free` are zeroed here on purpose. Those are global in
        `nix.settings`, and a store's auto-GC bookkeeping (`gcRunning`,
        `lastGCCheck`, `availAfterGC`) is PER PROCESS, so two daemons otherwise
        become two independent auto-GC actors: both decide to collect, the second
        computes its budget from its own stale free-space reading, and one waits
        on the store-global exclusive lock behind the other. Worse here than in
        general, because a GC triggered by CI runs at `Nice=19` inside this
        16G cgroup while holding that lock, so the user's own daemon blocks behind
        a deliberately deprioritised collection. Leaving exactly one auto-GC actor
        (the system daemon) avoids all of it; builds are unaffected either way
        since `addTempRoot` falls back to the gc-socket.
      '';
    };

    resources = lib.mkOption {
      type = lib.types.attrsOf (lib.types.oneOf [lib.types.str lib.types.int lib.types.bool]);
      default = {
        CPUWeight = 20;
        IOWeight = 20;
        Nice = 19;
        # MemoryHigh alone is NOT a bound: it applies reclaim pressure but
        # allows growth past the value. This host has swap
        # (hosts/desktop/hardware-configuration.nix:31-33), so without a hard
        # limit CI can still push into swap and reproduce the original stall —
        # which is the failure this whole module exists to prevent. So: throttle
        # at 12G, refuse past 16G, and forbid swapping the daemon's own work.
        #
        # The denominator, which this file used to omit: desktop has 32 GiB. So
        # 12G is reclaim pressure at 37.5% and 16G is a hard ceiling at exactly
        # half the machine, leaving the interactive user >=16 GiB guaranteed.
        # Sized against the real workload rather than a guess: bipa builds with
        # the dev profile and mold, `[profile.release]` sets no LTO, and its
        # ~4500 tests run as `cores`-bounded parallel processes rather than one
        # giant one, so a single >16G step is unlikely. If CI ever switches to
        # release + LTO, revisit this — an LTO link of that workspace can exceed
        # 16G and `OOMPolicy = "continue"` would make it fail quietly.
        MemoryHigh = "12G";
        MemoryMax = "16G";
        MemorySwapMax = 0;
        # a single build being OOM-killed must not take the daemon down
        OOMPolicy = "continue";
      };
      description = ''
        Resource controls for the CI daemon's cgroup. Unlike the same settings
        on `nix-daemon.service`, these only affect CI: `Nice`/`CPUWeight` make CI
        yield to anything interactive, `MemoryHigh` throttles by reclaim first,
        and `MemoryMax`/`MemorySwapMax` are the hard stops that keep a runaway
        build out of swap.

        Two honest caveats. `IOWeight` is very likely INERT on these hosts:
        cgroup-v2 `io.weight` is consumed only by BFQ or by the blk-iocost
        controller, and measured on this hardware the NVMe scheduler is `none`
        with `io.cost.qos`/`io.cost.model` empty, so the value is accepted and
        ignored. The real IO reduction comes from `max-substitution-jobs = 4`
        and the worker's `max-jobs = 1`. And a hard `MemoryMax` with
        `MemorySwapMax = 0` is a deliberate behaviour CHANGE, not just a bound: a
        build that previously finished slowly by swapping is now OOM-killed
        instead. That is the trade this module was written to make — it converts
        "the machine becomes unusable" into "the CI build fails" — but it is a
        trade, so it belongs in writing.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.sockets.nix-daemon-ci = {
      description = "Nix Daemon Socket (CI)";
      wantedBy = ["sockets.target"];
      socketConfig = {
        ListenStream = "${socketDir}/socket";
        SocketMode = "0660";
        SocketUser = "root";
        SocketGroup = cfg.group;
      };
    };

    systemd.services.nix-daemon-ci = {
      description = "Nix Daemon (CI)";
      requires = ["nix-daemon-ci.socket"];
      after = ["nix-daemon-ci.socket"];
      # ssh is needed for remote builders / git+ssh fetches, exactly as the
      # nixpkgs nix-daemon module provides it
      path = [config.nix.package config.programs.ssh.package];

      # a changed nix.conf must restart this daemon too, or it keeps serving
      # builds under stale settings
      restartTriggers = [config.environment.etc."nix/nix.conf".source];
      # keep the socket available across a switch, for the same reason nixpkgs
      # sets it on nix-daemon: clients do not retry a failed socket connect
      stopIfChanged = false;

      environment =
        config.nix.envVars
        // {
          CURL_CA_BUNDLE = config.security.pki.caBundle;
          NIX_CONFIG = cfg.nixConfig;
        }
        // config.networking.proxy.envVars;

      unitConfig = {
        # mirrors the unit nix ships: do not start before the store is usable.
        # The condition deliberately names upstream's path, not `socketDir`: the
        # point is to be skipped when /nix is read-only, and `socketDir` is on
        # tmpfs and gets created both by the tmpfiles rule below and by systemd's
        # own ListenStream parent-directory handling, so a condition on it is
        # unconditionally true and protects nothing. `RequiresMountsFor` only
        # guarantees the mounts exist, not that they are writable.
        RequiresMountsFor = ["/nix/store" "/nix/var" "/nix/var/nix/db"];
        ConditionPathIsReadWrite = "/nix/var/nix/daemon-socket";
      };

      serviceConfig =
        {
          # mirrors the unit nix itself ships, minus the socket it hardcodes
          ExecStart = "@${config.nix.package}/bin/nix-daemon nix-daemon --daemon";
          KillMode = "process";
          LimitNOFILE = 1048576;
          TasksMax = 1048576;
          Delegate = "";
        }
        // cfg.resources;
    };

    systemd.tmpfiles.rules = [
      "d ${socketDir} 0755 root ${cfg.group} - -"
    ];
  };
}
