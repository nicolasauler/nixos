# Self-hosted Nix binary cache (attic) — cachix-private replacement for bipa.
#
# Activate: add ../../modules/services/atticd.nix to the chosen host's
# configuration.nix imports, then before rebuilding:
#
#   sudo mkdir -p /etc/secrets
#   echo "ATTIC_SERVER_TOKEN_RS256_SECRET_BASE64=\"$(nix run nixpkgs#openssl -- genrsa -traditional 4096 | base64 -w0)\"" \
#     | sudo tee /etc/secrets/atticd.env >/dev/null
#   sudo chmod 600 /etc/secrets/atticd.env
#
# After `nixos-rebuild switch` (on the desktop):
#   atticd-atticadm make-token --sub nic --validity 1y --pull 'bipa*' \
#     --push 'bipa*' --create-cache 'bipa*'   # wrapper runs as the atticd user
#   attic login local http://<desktop-tailnet-name>:8080 <token>
#   attic cache create bipa
#   attic use bipa                      # per dev machine: wires substituter+key
#   attic push bipa <store-paths>      # or: attic watch-store bipa
#
# CI (Blacksmith) can't see the tailnet; expose ONLY this port when ready:
#   tailscale funnel 8080
# then give CI a push-scoped token as a secret.
{...}: {
  services.atticd = {
    enable = true;
    # nixpkgs option name for the RS256 secret file (older releases: credentialsFile)
    environmentFile = "/etc/secrets/atticd.env";
    settings = {
      listen = "[::]:8080";
      jwt = {};
      # sqlite database + local NAR storage under /var/lib/atticd
      storage = {
        type = "local";
        path = "/var/lib/atticd/storage";
      };
      # attic's recommended chunking defaults: global dedup across caches
      chunking = {
        nar-size-threshold = 65536;
        min-size = 16384;
        avg-size = 65536;
        max-size = 262144;
      };
      garbage-collection = {
        default-retention-period = "3 months";
      };
    };
  };

  # Reachable from the tailnet only; Funnel opts a port in explicitly later.
  networking.firewall.trustedInterfaces = ["tailscale0"];
}
