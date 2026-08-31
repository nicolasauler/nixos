{
  # Stand-in for the PRIVATE `certus-infra` input, used only in CI.
  #
  # This repo is public but one of its flake inputs is not
  # (git+ssh://git@github.com/nicolasauler/certus_infra.git), so a hosted runner
  # cannot fetch it without a deploy key. Rather than give CI a key, the
  # workflow overrides that input with this stub:
  #
  #   nix build --override-input certus-infra path:./ci/stub-certus ...
  #
  # That works because the checks CI runs were deliberately decoupled from it:
  # checks.buildbot-fanout takes buildbot-nix directly from its public upstream.
  # Nothing here is imported by anything; the stub exists so the flake's inputs
  # resolve without network access to a private repo.
  #
  # What this consciously puts OUT of CI's reach, because it genuinely needs the
  # private module: nixosConfigurations.desktop and checks.buildbot-workstation.
  # Those stay local-only until the buildbot modules move somewhere public.
  description = "CI stub for the private certus-infra input";
  outputs = _: {};
}
