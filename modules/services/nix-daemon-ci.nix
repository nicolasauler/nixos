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
      '';
      description = ''
        NIX_CONFIG for the CI daemon itself. This is where settings the
        untrusted client cannot set belong — `max-substitution-jobs` above all,
        which the trust gate keeps out of a client's reach.
      '';
    };

    resources = lib.mkOption {
      type = lib.types.attrsOf (lib.types.oneOf [lib.types.str lib.types.int lib.types.bool]);
      default = {
        CPUWeight = 20;
        IOWeight = 20;
        Nice = 19;
        MemoryHigh = "12G";
        OOMPolicy = "continue";
      };
      description = ''
        Resource controls for the CI daemon's cgroup. Unlike the same settings
        on `nix-daemon.service`, these only affect CI: `Nice`/`CPUWeight`/
        `IOWeight` make CI yield to anything interactive, and `MemoryHigh`
        throttles by reclaim rather than by the OOM killer.
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
      path = [config.nix.package];

      environment =
        config.nix.envVars
        // {
          CURL_CA_BUNDLE = config.security.pki.caBundle;
          NIX_CONFIG = cfg.nixConfig;
        }
        // config.networking.proxy.envVars;

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
