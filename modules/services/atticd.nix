# Self-hosted Nix binary cache (attic) — the private-cachix replacement for bipa.
#
# PRIVATE, TAILNET-ONLY. Reachable by:
#   - nic@desktop (this host), nic@precision
#   - one coworker, whose machine gets `desktop` shared into his tailnet
# Nothing is exposed to the internet: use `tailscale serve`, never `funnel`.
#
#   sudo tailscale serve --bg --https 8443 http://127.0.0.1:8080
#
# That publishes https://desktop.tailb7fb5e.ts.net:8443/ to tailnet peers only
# (8443, not 443: buildbot-nix's webhook funnel owns 443 -> 8010 here), with a
# real cert fetched by tailscaled (this is what upstream's "put it behind nginx
# for HTTPS" recommendation asks for). `serve` config persists in tailscaled
# state, so it survives reboots without a unit here. Verify with:
#   tailscale serve status     # must NOT say "Funnel on"
#
# One-time, before the first rebuild of this host. These are written for BASH
# deliberately, and that matters here: this host's owner uses nushell, where
# `$(...)` inside double quotes is NOT substituted, a trailing `\` is NOT a line
# continuation, and `>/dev/null` in that position is passed as an argument rather
# than a redirection. Run them under bash, or use the nushell forms below - if you
# paste the bash form into nu, `atticd.env` receives the literal text
# `$(nix run nixpkgs#openssl ...)` instead of a key, with NO error, and atticd
# then starts with a garbage signing secret.
#
#   bash:
#     sudo mkdir -p /etc/secrets
#     echo "ATTIC_SERVER_TOKEN_RS256_SECRET_BASE64=\"$(nix run nixpkgs#openssl -- genrsa -traditional 4096 | base64 -w0)\"" | sudo tee /etc/secrets/atticd.env > /dev/null
#     sudo chmod 600 /etc/secrets/atticd.env
#
#   nushell (measured on nu 0.115.0):
#     sudo mkdir -p /etc/secrets
#     let key = (nix run nixpkgs#openssl -- genrsa -traditional 4096 | base64 -w0 | str trim)
#     $'ATTIC_SERVER_TOKEN_RS256_SECRET_BASE64="($key)"' | sudo tee /etc/secrets/atticd.env | ignore
#     sudo chmod 600 /etc/secrets/atticd.env
#
#   `sudo tee` is fine there because nu resolves `sudo` and passes `tee` as an
#   argument. A bare `| tee /path` is NOT: nu has its own `tee` builtin that
#   expects a closure, so it fails with "expected block, closure or record".
#   Reach for `^tee` if you ever drop the sudo.
#
# Verify either way before rebuilding — the value must be one long base64 blob,
# and must not contain the characters `$` or `(`:
#
#   sudo grep -c '\$(' /etc/secrets/atticd.env      # must print 0
#
# After `nixos-rebuild switch` (the module installs the atticd-atticadm wrapper,
# which runs atticadm as the atticd user, plus the `attic` client):
#
#   sudo atticd-atticadm make-token --sub nic --validity 1y --pull 'bipa*' --push 'bipa*' --create-cache 'bipa*'
#   attic login bipa https://desktop.tailb7fb5e.ts.net:8443/ <token>
#   attic cache create bipa
#   attic use bipa        # per machine: writes substituter + public key + token
#                         # into ~/.config/nix/nix.conf
#
# Onboarding the coworker (two independent grants — both are required):
#   1. Network: share this node with him in the Tailscale admin console
#      (Machines -> desktop -> Share). Sharing one node beats adding him to the
#      tailnet: he gets `desktop` and nothing else, and MagicDNS resolves
#      desktop.tailb7fb5e.ts.net for him.
#   2. Cache: mint him his own token — no pubkey needed, attic authenticates with
#      signed JWTs, and the NAR signing key never leaves this host:
#        sudo atticd-atticadm make-token --sub <his-name> --validity 90d --pull bipa
#      Add --push bipa only if he should upload too. Then, on his machine:
#        attic login bipa https://desktop.tailb7fb5e.ts.net:8443/ <token>
#        attic use bipa
#      Revoking means rotating: tokens are stateless, so a leaked one is valid
#      until it expires (hence 90d, not 1y) or until the RS256 secret is
#      replaced, which invalidates every token including yours.
#
# `attic use` only takes effect for users in `nix.settings.trusted-users`
# (already ["nic"] on every host here).
#
# bipa's CI cannot reach the tailnet, so it is deliberately NOT wired up: that
# would need Funnel (public) or a runner inside the tailnet. Separate decision.
{pkgs, ...}: let
  # MagicDNS name of this host, as published by `tailscale serve`.
  fqdn = "desktop.tailb7fb5e.ts.net";
in {
  services.atticd = {
    enable = true;
    # nixpkgs option name for the RS256 secret file (older releases: credentialsFile)
    environmentFile = "/etc/secrets/atticd.env";
    settings = {
      listen = "[::]:8080";

      # Upstream's config-template.toml marks both of these "_must_ be
      # configured for production use":
      #   - an unset api-endpoint is synthesized from the client's Host header,
      #   - an empty allowed-hosts accepts every Host header.
      # The trailing slash on api-endpoint is required by attic.
      api-endpoint = "https://${fqdn}:8443/";
      allowed-hosts = [
        "${fqdn}:8443" # via `tailscale serve` (TLS terminated by tailscaled)
        "desktop:8080" # direct over the tailnet
        "${fqdn}:8080" # direct, fully qualified
        "localhost:8080"
      ];

      jwt = {};

      # sqlite database (module default, /var/lib/atticd/server.db) + local NARs
      storage = {
        type = "local";
        path = "/var/lib/atticd/storage";
      };

      # Upstream's recommended chunking defaults, for global dedup across caches.
      # Changing any value strands existing chunks (the cutpoints move), so the
      # dedup ratio suffers until everything is re-uploaded: treat as fixed.
      chunking = {
        nar-size-threshold = 65536; # 64 KiB
        min-size = 16384; # 16 KiB
        avg-size = 65536; # 64 KiB
        max-size = 262144; # 256 KiB
      };

      compression.type = "zstd";

      garbage-collection = {
        interval = "12 hours";
        # LRU: an object survives only if something *fetched* it inside this
        # window, so a week keeps the working set (current master plus whatever
        # branches the three of us build) and drops the rest.
        # Per-cache override: `attic cache configure bipa --retention-period …`
        default-retention-period = "1 week";
      };
    };
  };

  # Only attic's ports, and only on the tailnet interface. Deliberately not
  # `trustedInterfaces = ["tailscale0"]`: a shared node has no business reaching
  # every other port on this machine, and ssh stays governed by the nftables
  # rule in the host config (precision's address only).
  networking.firewall.interfaces."tailscale0".allowedTCPPorts = [
    8443 # `tailscale serve` listener (443 belongs to buildbot's webhook funnel)
    8080 # atticd itself, for direct http://desktop:8080 use
  ];

  # `attic` (client) for make-token/login/cache-create on the server itself.
  # Laptops that only pull need the same package: add `attic-client` to their
  # environment.systemPackages — they must NOT import this module.
  environment.systemPackages = [pkgs.attic-client];
}
