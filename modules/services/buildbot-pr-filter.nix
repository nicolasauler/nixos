# Restrict which changes this workstation is willing to build.
#
# THE PROBLEM. buildbot-nix builds every pull request on every registered repo
# ("build all pull requests" in project_config.py — no author filter, and no
# NixOS option for one), and additionally builds every push to the default
# branch (models.py `do_run()` returns true for the default branch
# unconditionally, whatever `master.branches` says). For a shared repo like
# bipa-app/bipa that is a firehose: one event fans out to five heavy checks,
# and this box measured ~20-40 min per check with four running concurrently.
# Commissioning it as a PR gate for the whole team took the machine down.
#
# THE FIX. A patch adding two env-driven knobs to the schedulers, each
# defaulting to upstream behaviour so the patch is safe to carry:
#
#   BUILDBOT_PR_AUTHORS     comma-separated forge logins allowed to trigger
#                           pull-request builds. Empty/unset = build every PR.
#   BUILDBOT_BUILD_PUSHES   "0"/"false"/"no"/"off" = do not build pushes at all
#                           (default branch, `branches` globs, merge queue).
#                           Unset = build them, as upstream does.
#
# Pull-request changes carry the bare forge login as the change author
# ("nicolasauler"); push changes carry a git identity ("Nicolas Auler
# <nicolas@example.com>"). The two are not interchangeable, which is why the
# push path is a boolean rather than a second allowlist.
#
# WHY NOT THE RUNTIME TOGGLE. buildbot stores an `enabled` flag per scheduler
# in postgres, so `-prs` can be disabled through the UI/API instead. That state
# is invisible from this repo, is not reproduced by a rebuild, and a reconfig
# that recreates schedulers can quietly restore it. This is the declarative
# equivalent, and adding a collaborator later is a one-word change here.
#
# Both variables are read once at import time by the master, so changing them
# needs `systemctl restart buildbot-master` (a rebuild does that anyway).
{options, ...}: {
  # Patch the module's own default package, so the python scope and the
  # buildbot-gitea wiring stay exactly as upstream computed them.
  services.buildbot-nix.packages.buildbot-nix =
    options.services.buildbot-nix.packages.buildbot-nix.default.overrideAttrs
    (old: {
      patches = (old.patches or []) ++ [./buildbot-nix-pr-author-filter.patch];
    });

  systemd.services.buildbot-master.environment = {
    BUILDBOT_PR_AUTHORS = "nicolasauler";
    BUILDBOT_BUILD_PUSHES = "0";
  };
}
