# agenix recipient map, consumed by the `agenix` CLI (not by the NixOS modules --
# those read ./secrets/*.age directly). Each file is encrypted to EVERY key in its
# `publicKeys`; adding a recipient means `agenix --rekey`.
#
# TWO recipients, deliberately:
#   * the desktop HOST key, because the machine decrypts these at activation with
#     no human present.
#   * an ADMIN key, because a host key is not a backup. Reinstall desktop, or lose
#     its /etc/ssh, and anything encrypted only to it is unrecoverable. Keep this
#     private key backed up somewhere that is not desktop and not this repo.
#
# PROVENANCE of the desktop key: the `desktop` line in ~/.ssh/known_hosts,
# SHA256:V42e/MrKxjQgVkOhcAYkFCrc3NlWv7X6oMrDIdKLSgw, established across earlier
# authenticated sessions. `ssh-keyscan -t ed25519 desktop` re-derives it when the
# host is up, but do not treat that as verification: it needs no authentication,
# and on a tailnet the name->key binding comes from the coordination server, so
# whoever controls the tailnet account could point `desktop` at a machine of
# theirs and hand you a recipient that can read these files. Compare against the
# pin or an out-of-band fingerprint, not against whatever answers port 22.
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
# to contain the password too. Current shape, so you can restore it without
# guessing:
#
#   [
#     { "name": "desktop", "pass": "<64 hex chars>", "cores": 1 }
#   ]
#
# NOTHING MACHINE-CHECKS THAT `cores`. It is inside a ciphertext, so no test and
# no eval can read it, and `age` does not pad, so even the file size cannot tell
# 1 from 8 (both are one character). certus-infra sets 8, this repo forces the
# worker process count to 1, and a stale 8 in here would leave desktop-001..007
# permanently unattached -- visible in the master's worker list, harmless, and
# silent otherwise. Confirm after a switch:
#
#   curl -s http://desktop:8010/api/v2/workers | grep -c desktop-00
let
  # One key per line, and NEVER paste ssh-keyscan's output verbatim: it prefixes a
  # `# host:22 SSH-2.0-...` banner line, and agenix splits publicKeys by LINE
  # (pkgs/agenix.sh pipes them through `jq -r .[]` into a `read -r` loop), so a
  # banner inside the string becomes its own `--recipient` and age aborts with
  # "unknown recipient type". That breaks `agenix -e` and `-r` while leaving
  # decryption working, so it hides until the day you try to rotate.
  admin = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOvd+vu/N2gNxGjdAbEnzV7sUH1zmCYpQW61LzY6WiEj nickvarauler@gmail.com";
  desktop = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAfEpsxGGyd1NMQawlfASvcpPUutwMw33ODAKml2TKxB";
  both = [admin desktop];
in {
  "secrets/buildbot-workers.json.age".publicKeys = both;
  "secrets/buildbot-worker-password.age".publicKeys = both;
}
