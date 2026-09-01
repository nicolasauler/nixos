# agenix recipient map, consumed by the `agenix` CLI (not by the NixOS modules --
# those read ./secrets/*.age directly). Each file is encrypted to EVERY key in its
# `publicKeys`; adding a recipient means `agenix --rekey`.
#
# TWO recipients, deliberately:
#   * the desktop HOST key, because the machine decrypts these at activation with
#     no human present. Taken from the live host, so it is the key desktop
#     actually has: `ssh-keyscan -t ed25519 desktop`.
#   * an ADMIN key, because a host key is not a backup. Reinstall desktop, or lose
#     its /etc/ssh, and anything encrypted only to it is unrecoverable. Keep this
#     private key backed up somewhere that is not desktop and not this repo.
#
# Editing or rotating, from the repo root, with the admin key at
# ~/.ssh/id_ed25519 or in ssh-agent:
#
#   nix run github:ryantm/agenix -- -e secrets/buildbot-worker-password.age
#
# ROTATING THE WORKER PASSWORD TOUCHES BOTH FILES. buildbot checks the worker's
# password against the master's worker list, so the value in
# buildbot-worker-password.age must equal the "pass" field in
# buildbot-workers.json.age. Change one and the worker cannot attach.
#
# The JSON also carries `cores`, which is capacity policy rather than a secret,
# and it must stay equal to `services.buildbot-nix.worker.workers` in
# modules/services/buildbot-limits.nix -- one slot per core. It lives in the
# encrypted file only because buildbot reads a single JSON document that happens
# to contain the password too.
let
  admin = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOvd+vu/N2gNxGjdAbEnzV7sUH1zmCYpQW61LzY6WiEj nickvarauler@gmail.com";
  desktop = "desktop:22 SSH-2.0-OpenSSH_10.5
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAfEpsxGGyd1NMQawlfASvcpPUutwMw33ODAKml2TKxB";
  both = [admin desktop];
in {
  "secrets/buildbot-workers.json.age".publicKeys = both;
  "secrets/buildbot-worker-password.age".publicKeys = both;
}
