# buildbot-nix x bipa — the desktop-side plumbing that lets the existing
# buildbot instance (imported in this host's configuration.nix) build
# bipa-app/bipa's flake checks.
#
# Why this file exists at all — three different fetches, three credentials:
#   1. buildbot CLONES bipa      -> GitHub App installation token (buildbot-nix
#                                   mints these itself; nothing to configure).
#   2. nix EVALUATES bipa's flake -> the flake has a PRIVATE input
#      (git+https agent-sdk). Flake inputs are fetched by the evaluating
#      process — nix-eval-jobs running as the `buildbot-worker` user — using
#      the system nix.conf. buildbot-nix deliberately does not manage this
#      (verified at the pinned rev: no netrc/access-tokens anywhere in its
#      modules or python; the App token never reaches nix's fetchers).
#   3. Derivations BUILD          -> pure/sandboxed, no credentials by design.
#
# So (2) is ours. `access-tokens` is nix's documented knob for authenticated
# GitHub fetches (github: refs and git+https to github.com), but its value is
# a secret and everything in `nix.settings`/`extraOptions` is rendered into
# /etc/nix/nix.conf — a world-readable file in the store. The standard NixOS
# idiom is therefore an include directive pointing at a file outside the
# store. `!include` (vs `include`) skips silently when the file is missing or
# unreadable — required here, because every user's every nix invocation parses
# nix.conf, and a loud include on a group-restricted file would break nix for
# everyone else.
#
# The flip side of that silence: WRONG PERMISSIONS FAIL SILENTLY (the fetch
# just 404s). The consumer is the buildbot-worker user, NOT root, so:
#
#   umask 027
#   echo 'access-tokens = github.com=<PAT: contents:read on bipa-app/bipa and bipa-app/agent-sdk>' \
#     | sudo tee /etc/secrets/nix-access-tokens.conf >/dev/null
#   sudo chown root:buildbot-worker /etc/secrets/nix-access-tokens.conf
#   sudo chmod 640 /etc/secrets/nix-access-tokens.conf
#
# (Your own `nic` builds of bipa keep using your gh/ssh credentials; add nic
# to the buildbot-worker group only if you want them flowing through the same
# token.)
#
# Upgrade path, not a blocker: sops-nix/agenix would template this same file
# with a managed lifecycle; both are currently commented out in this flake.
{...}: {
  nix.extraOptions = ''
    !include /etc/secrets/nix-access-tokens.conf
  '';
}
