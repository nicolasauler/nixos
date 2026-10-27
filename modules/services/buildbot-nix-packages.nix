# Fixes for packages of the pinned buildbot-nix (flake.nix) that no longer build
# against this nixpkgs. Upstream is being archived (its latest merge, 2026-10-24, is
# the archive notice) and still carries the bug, so it is fixed here, on top of the
# module's own default, rather than by moving the input.
#
# buildbot-effects: packages/buildbot-effects.nix passes makeWrapperArgs as ONE
# string, "--prefix PATH : <bubblewrap>/bin". The build now runs with structured
# attrs ("structuredAttrs is enabled" in its log), so that element reaches makeWrapper
# as a single argument and it dies with "makeWrapper doesn't understand the arg
# --prefix PATH : ...". Same wrapper, as separate arguments.
#
# Import this wherever buildbot-nix's master or worker module is imported, directly
# or through certus-infra's buildbot module: the option only exists there.
{
  lib,
  options,
  pkgs,
  ...
}: {
  services.buildbot-nix.packages.buildbot-effects = options.services.buildbot-nix.packages.buildbot-effects.default.overridePythonAttrs (_: {
    makeWrapperArgs = ["--prefix" "PATH" ":" (lib.makeBinPath [pkgs.bubblewrap])];
  });
}
