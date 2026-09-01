# nixos

Fresh clone, in order. The first step is not optional: the pre-commit hook below
runs `alejandra`, which comes from the flake's devShell and is in no host's
`systemPackages`, so skipping it makes every commit fail.

```
direnv allow                     # loads .envrc -> `use flake` -> the devShell
curl -LO https://github.com/pre-commit/pre-commit/releases/download/v4.0.1/pre-commit-4.0.1.pyz
python pre-commit-4.0.1.pyz install
```

The devShell provides `alejandra`, `python3`, `actionlint`, `cachix` and `agenix`,
and arms the `nicnixos` binary cache for commands run inside it. Read the push
warning in `flake.nix` before using `cachix` — pushing the wrong closure publishes
secrets to a public cache.

## Secrets

`secrets/*.age`, encrypted with [agenix]. Reading the repo, building it, and
switching a host need nothing from you — desktop decrypts at activation with its
own `/etc/ssh/ssh_host_ed25519_key`.

Editing one needs the admin key at `~/.ssh/id_ed25519`, and must be run from the
repo root, because `agenix` resolves recipients from `./secrets.nix`:

```
agenix -e secrets/buildbot-worker-password.age
```

Every file is encrypted to **two** recipients: desktop's host key, so activation
works unattended, and an admin key, because a host key is not a backup — reinstall
the machine and anything encrypted only to it is gone. Keep that private key
somewhere that is not desktop and not this repo.

**The buildbot worker password lives in two of these files and they must agree.**
The master reads its worker list from `buildbot-workers.json.age`; the worker reads
`buildbot-worker-password.age`. Change one without the other and the worker is
rejected with `UnauthorizedLogin` while still appearing in the master's worker list.

**Nothing checks that for you.** `checks.buildbot-workstation` proves the
master/worker pairing *mechanism* — a diverged pair does fail it — but it supplies
its own dummy credentials and never reads `secrets/*.age`, so it passes whatever
those two files contain. Agreement of the deployed halves is verified by hand, and
the symptom is a worker that will not attach after the switch.

The previous password was a plaintext literal committed to this public repo, so it
is treated as burned rather than moved: `secrets/` carries a new value, and the old
one should be considered known to everyone. See `secrets.nix` for the rotation
procedure and what else it touches.

[agenix]: https://github.com/ryantm/agenix

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
