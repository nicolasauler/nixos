{
  description = "Nixos config flake";

  # NO `nixConfig` here, deliberately, and the reason is worth keeping because I
  # got it wrong once and shipped the wrong comment.
  #
  # A flake `nixConfig` looks like it scopes a substituter to this repo. It does
  # not. Nix prompts once per (setting, VALUE) pair and then persists the answer
  # user-globally in ~/.local/share/nix/trusted-settings.json — not per flake. So
  # accepting it once arms the cache for every subsequent `nix` command against
  # this flake, including `nixos-rebuild switch --flake .#precision` (whose
  # nixos-system-* closure cache.nixos.org does not carry, so a forged toplevel
  # would be accepted), AND for any OTHER flake that names the same substituter
  # URL. Measured on this machine after a single acceptance: `nix eval` from an
  # unrelated directory printed "Using saved setting for 'extra-substituters =
  # https://nicnixos.cachix.org'". That is exactly the machine-wide trust this
  # repo decided not to grant.
  #
  # It also bought nothing in CI: cachix-action runs `cachix use <name>` itself
  # before any build step, writing the substituter and key into the runner's
  # nix.conf (gated only by `skipAddingSubstituter`, which we leave at false).
  #
  # So the cache is armed in exactly one place: the devShell's own NIX_CONFIG
  # below. On a machine that has never accepted it, a plain `nix build` against
  # this flake correctly gets nothing — verified with a fresh HOME, which printed
  # "ignoring untrusted flake configuration setting 'extra-substituters'".
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-index-database = {
      url = "github:nix-community/nix-index-database";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix-colors.url = "github:misterio77/nix-colors";
    # stylix.url = "github:danth/stylix";

    hyprland.url = "github:hyprwm/Hyprland";

    nixvim = {
      url = "github:nix-community/nixvim";
      # If using a stable channel you can use `url = "github:nix-community/nixvim/nixos-<version>"`
      # no nixpkgs follows: nixvim recommends building against its own tested nixpkgs pin
    };

    # sops-nix.url = "github:Mic92/sops-nix";
    # agenix.url = "github:ryantm/agenix";

    # Pinned: 54f75733 (2026-08-10) moved the bind mounts from fileSystems to
    # systemd.mounts with After=sentinelone-init.service. That service is ordered
    # after sysinit.target, which is after local-fs.target, which pulls in the mount
    # itself -- so activation dies with "Transaction order is cyclic" and systemd
    # drops local-fs.target. Unpin once upstream breaks that loop.
    sentinelone.url = "github:devusb/sentinelone-nix/1e58fdcd464ef0b3929c5fd181c837d2d8eaf0d3";

    certus-infra = {
      url = "git+ssh://git@github.com/nicolasauler/certus_infra.git";
      # One source of truth: the private module and the public VM check use the
      # same buildbot-nix revision and API.
      inputs.buildbot-nix.follows = "buildbot-nix";
    };

    # Public direct input: checks.buildbot-fanout can evaluate without fetching
    # certus-infra. Keep the revision explicit; follows above gives the private
    # module this exact source too.
    buildbot-nix = {
      url = "github:nix-community/buildbot-nix/19a89fd4c890433dab7062672ff95efe0128db3c";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # certus-infra.url = "git+file:///home/nic/certus/certus_infra.git/main";
  };

  outputs = {
    self,
    nixpkgs,
    ...
  } @ inputs: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [];
      config = {
        allowUnfree = true;
        # cudaSupport = true;
      };
    };
  in {
    # Replaces the old shell.nix, which did `import <nixpkgs> {}` — the channel,
    # so it was unpinned and impure and had nothing to do with the nixpkgs these
    # hosts are actually built from. This shell uses the locked input.
    #
    # It is also the right scope for the cache, but via the `NIX_CONFIG` attribute
    # below and NOT via a flake `nixConfig` — see the top of this file for why that
    # would not have scoped anything. Consequence worth knowing: `nix print-dev-env`
    # exports NIX_CONFIG and nix-direnv sources it, so every process started from
    # this directory inherits the cache, including `nixos-rebuild switch --flake
    # .#precision`, whose nixos-system-* closure cache.nixos.org cannot contradict.
    # That is a much narrower scope than user-global-and-permanent, but the most
    # security-relevant consumer is still inside it. To switch without the cache:
    # `NIX_CONFIG= nixos-rebuild switch --flake .#precision`, or run it from
    # outside the checkout.
    devShells.${system}.default = pkgs.mkShell {
      packages = with pkgs; [
        alejandra # formatter this repo is written with
        python3 # the VM tests' testScript is Python
        # Pushing to nicnixos. READ THE WARNING BELOW BEFORE YOU DO.
        cachix
        actionlint # .github/workflows/ci.yaml is a required-check surface
      ];

      # This — not a machine-wide substituter — is what makes the cache live
      # locally. It applies to `nix` commands run inside this shell, which direnv
      # enters on cd (`nix print-dev-env` exports NIX_CONFIG, so nix-direnv picks
      # it up), and to nothing else: nicnixos only ever holds what this repo
      # builds, so putting it in `nix.settings` would add a pointless lookup to
      # every unrelated `nix build` on the machine.
      #
      # NEVER put a secret in this attribute. It is a full nix config surface —
      # `access-tokens`, `netrc-file`, `post-build-hook` all live here — and
      # mkShell env attrs land VERBATIM in the devShell's derivation, which is
      # world-readable in the store (mode 444) and gets pushed to the PUBLIC cache
      # like any other built path. A URL and a public key are fine. An
      # `access-tokens = github.com=…` for the private certus-infra input would be
      # published. Note also that these are assignments, not appends: entering this
      # shell REPLACES an inherited NIX_CONFIG rather than extending it.
      NIX_CONFIG = ''
        extra-substituters = https://nicnixos.cachix.org
        extra-trusted-public-keys = nicnixos.cachix.org-1:360nRdjlB+ydcwCGF3V17ojXcvyWqz/SJ3hTarX6Pqs=
      '';

      # PUSHING, and the warning belongs HERE rather than only in ci.yaml, because
      # this is the path a human actually takes and ci.yaml is not what they are
      # reading when they take it.
      #
      #   cachix watch-exec nicnixos -- nix build .#checks.x86_64-linux.buildbot-fanout
      #
      # The rule is about the CLOSURE, not the subcommand. `watch-exec` pushes what
      # that one command realises; `cachix push nicnixos <path>` pushes what you
      # name; and `cachix watch-store nicnixos` takes no installable at all and
      # pushes everything the machine builds while it runs — so starting it and then
      # running any `nixos-rebuild`, or the local `checks.buildbot-workstation`
      # verification that ci.yaml mentions, publishes those closures. Never use
      # `watch-store` against this cache on a machine that builds certus closures,
      # which is all of them.
      #
      # What must never reach a PUBLIC cache: `checks.buildbot-workstation` and any
      # `nixosConfigurations.*.config.system.build.toplevel`. certus-infra puts the
      # buildbot worker password and webhook secret in the store via
      # `pkgs.writeText`. Measured on this machine:
      # /nix/store/w8bvnl77ssir0g4ikvzar5aj0sjwmnhz-workers.json is mode 444 and
      # contains the literal `certus-worker-local`, and is reachable from 169 paths
      # in its referrers closure (19 direct referrers) including
      # nixos-test-driver-buildbot-workstation and nixos-system-*. One push over any
      # of those publishes it permanently.
      #
      # `checks.buildbot-fanout` is safe to push, but note WHY: it is safe by
      # VALUE, not by structure — its writeText passwords are dummies
      # (`tests/buildbot-fanout.nix:38,56`, "test-password"). Replace one with a
      # real credential and every push of this check publishes it to a PUBLIC
      # cache, permanently, with no other signal.
    };

    # Adding a check here? Two things to know before you also wire it into CI or
    # push it to the cache:
    #   - `buildbot-workstation` needs the PRIVATE certus-infra input, so it
    #     cannot run on a public runner, and its closure carries certus's
    #     `writeText` secrets (the worker password and webhook secret). It must
    #     never be built by a job or command that pushes to nicnixos.
    #   - `buildbot-fanout` and `nix-substitution-limit` are the public ones, and
    #     they are safe by VALUE not by structure: fanout's writeText passwords are
    #     dummies, and the substitution node has no secret-shaped values at all
    #     (an unsigned local cache on loopback). Keep them that way.
    # The pushing side of this is enforced in .github/workflows/ci.yaml and
    # explained next to the devShell's push instructions above.
    checks.${system} = {
      buildbot-workstation = import ./tests/buildbot-workstation.nix {
        inherit pkgs inputs;
      };
      buildbot-fanout = import ./tests/buildbot-fanout.nix {
        inherit pkgs inputs;
      };
      # Takes only `pkgs`: it imports ./modules/services/nix-daemon-ci.nix
      # directly and needs no flake input, private or otherwise.
      nix-substitution-limit = import ./tests/nix-substitution-limit.nix {
        inherit pkgs;
      };
    };

    nixosConfigurations = {
      notebook = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/notebook/configuration.nix
        ];
      };
      desktop = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          # inputs.stylix.nixosModules.stylix
          inputs.certus-infra.nixosModules.observability
          inputs.certus-infra.nixosModules.buildbot
          # bencher: disabled until certus-infra bumps it — its importCargoLock
          # fetches async-stripe-1.0.0-rc.3 from the legacy crates.io download
          # URL, which now returns 403 and blocks every desktop rebuild.
          # inputs.certus-infra.nixosModules.bencher
          ./hosts/desktop/configuration.nix
        ];
      };
      xpsbipa = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/xpsbipa/configuration.nix
        ];
      };
      precision = nixpkgs.lib.nixosSystem {
        inherit pkgs;
        specialArgs = {inherit inputs;};
        modules = [
          ./hosts/precision/configuration.nix
        ];
      };
    };
  };
}
