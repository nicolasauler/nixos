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

Do not push a check's OUTPUT by hand. A check's output is its verdict, so once it
is in a trusted signed cache `nix build` substitutes it and the test never runs
again for that closure. That happened once here: a hand-pushed
`vm-test-run-buildbot-fanout` took the CI job from 9m20s to 39s with zero subtests
executed. The cache has since been purged, and two belts now prevent a recurrence
— CI realises the check with `--no-substitute`, and `pushFilter` keeps
`*-test-run-*` out of the cache — but there is no `cachix` CLI subcommand for
deleting a path, so a mistake here needs the dashboard to undo.
