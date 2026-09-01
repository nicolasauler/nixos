# The buildbot credentials, out of the world-readable Nix store.
#
# THE PROBLEM. buildbot authenticates its worker against the master's worker
# list, so both sides need the same password, and both sides were getting it from
# `pkgs.writeText`. A store path is world-readable by every user on the machine
# and, worse, it travels: it is part of the system closure, so anything that
# publishes that closure publishes the password. This repo is public and the
# value was committed in it, which is why the fix is a NEW password rather than a
# relocation of the old one — see the rotation note in ../../secrets.nix.
#
# WHY THIS IS A SEPARATE MODULE from buildbot-limits.nix, which is where the
# `workersFile` line used to live: that module is imported by
# ../../tests/buildbot-workstation.nix. A VM test node has no key that can
# decrypt anything, and it must not carry a real credential in its closure
# either. Capacity policy is reusable; secrets are host-scoped. So the test now
# supplies its own dummy pair by value, and desktop gets these.
#
# NO owner/group/mode SET HERE, AND THAT IS THE POINT. agenix defaults to
# owner "0", group "0", mode "0400" (measured on this config), i.e. root-only.
# Both consumers are systemd credentials rather than direct reads:
#
#   nixosModules/master.nix:1013  LoadCredential = "buildbot-nix-workers:${cfg.workersFile}"
#   nixosModules/worker.nix:133   LoadCredential = "worker-password-file:${cfg.workerPasswordFile}"
#
# systemd opens the source path as root while setting the unit up and re-exposes
# the contents inside that unit's $CREDENTIALS_DIRECTORY. Handing `buildbot` or
# `buildbot-worker` direct access to the plaintext would widen the blast radius
# for nothing.
#
# FAILURE MODE, deliberately loud: if agenix cannot decrypt (wrong host key,
# missing /etc/ssh/ssh_host_ed25519_key), the path does not appear, LoadCredential
# fails, and the unit refuses to start. There is no silent fallback to a default
# password.
{
  config,
  lib,
  ...
}: {
  age.secrets = {
    buildbot-workers.file = ../../secrets/buildbot-workers.json.age;
    buildbot-worker-password.file = ../../secrets/buildbot-worker-password.age;
  };

  # mkForce on both: certus-infra sets each to a `writeText` store path, and the
  # private module is the one place this repo cannot edit.
  services.buildbot-nix = {
    master.workersFile = lib.mkForce config.age.secrets.buildbot-workers.path;
    worker.workerPasswordFile = lib.mkForce config.age.secrets.buildbot-worker-password.path;
  };
}
