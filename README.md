# nixos

Fresh clone, in order. The first step is not optional: the pre-commit hook below
runs `alejandra`, which comes from the flake's devShell and is in no host's
`systemPackages`, so skipping it makes every commit fail.

```
direnv allow                     # loads .envrc -> `use flake` -> the devShell
curl -LO https://github.com/pre-commit/pre-commit/releases/download/v4.0.1/pre-commit-4.0.1.pyz
python pre-commit-4.0.1.pyz install
```

The devShell provides `alejandra`, `python3`, `actionlint` and `cachix`, and arms
the `nicnixos` binary cache for commands run inside it. Read the push warning in
`flake.nix` before using `cachix` — pushing the wrong closure publishes secrets to
a public cache.

## Binary cache

Pull needs nothing: it is armed by the devShell. Push happens from CI on merged
code. If you ever need to push by hand, push only `checks.buildbot-fanout`, and
read `flake.nix` first.

One outstanding owner action, recorded because CI cannot do it: an empty
`vm-test-run-buildbot-fanout` verdict was hand-pushed to the cache before
`pushFilter` existed. The CI step no longer trusts it (it realises the check with
`--no-substitute`), so it is no longer load-bearing — but it should be deleted from
the cache via the cachix dashboard so it stops occupying a warm GC slot and cannot
mislead a local `nix build`. There is no `cachix` CLI subcommand for deleting a
path.
