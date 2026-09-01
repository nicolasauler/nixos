# nixos

NixOS configs for four machines, plus the CI that keeps them from rotting.

| flake attr | what it is | `nh` | in CI |
|---|---|---|---|
| `precision` | Meteor Lake laptop, the daily driver | yes | evaluated |
| `xpsbipa` | Intel laptop, the work machine | yes | evaluated |
| `notebook` | Intel laptop. Note its `hostName` is `nixos`, not `notebook` | **no** | evaluated |
| `desktop` | AMD workstation; runs the buildbot that builds work repos | yes | **not** — see below |

```
nixos-rebuild switch --flake .#<attr>    # works everywhere
nh os switch -a                          # only where the table says yes
```

The flake attr is what you pass to `--flake`, and it is not always the machine's
hostname — see `notebook` above.

## Fresh clone, in order

The first step is not optional: the pre-commit hook below runs `alejandra`, which
comes from the flake's devShell and is in no host's `systemPackages`, so skipping it
makes every commit fail.

```
direnv allow                     # loads .envrc -> `use flake` -> the devShell
curl -LO https://github.com/pre-commit/pre-commit/releases/download/v4.0.1/pre-commit-4.0.1.pyz
python pre-commit-4.0.1.pyz install
```

The devShell provides `alejandra`, `python3`, `actionlint`, `cachix` and `agenix`,
and arms the `nicnixos` binary cache for commands run inside it. Read the push
warning in `flake.nix` before using `cachix` — pushing the wrong closure publishes
secrets to a public cache.

`precision` additionally needs a one-time step per machine and per version bump,
because the SentinelOne agent is a proprietary `.deb` that cannot be fetched:

```
nix-store --add-fixed sha256 /home/nic/bipa/SentinelAgent_linux_x86_64_v25_2_1_20.deb
nix build --out-link ~/.cache/gcroots/sentinelone-deb \
  '/home/nic/nixos#nixosConfigurations.precision.config.services.sentinelone.package.src'
```

The second command is not redundant. `--add-fixed` creates no GC root, and
`programs.nh.clean` runs weekly, so without it the store entry disappears and the
next rebuild fails. Two traps live here: `--add-fixed --add-root` silently ignores
the root flag, and modern `nix store add` defaults to recursive hashing while
`requireFile` expects flat — same file, different hash.

## Secrets

`secrets/*.age`, encrypted with [agenix]. Reading the repo, building it, and
switching a host need nothing from you — desktop decrypts at activation with its own
`/etc/ssh/ssh_host_ed25519_key`.

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

Still plaintext, each needing its own change: the buildbot webhook secret and the
Grafana password (both in the private `certus-infra` repo), the GitHub App key and
OAuth secret (hand-created on desktop), and the SentinelOne token. The webhook
secret is the one that matters, because that funnel is internet-facing and its
hostname is already published.

Two more live in **this** repo, so do not read `secrets/` as meaning it is clean:
`security.admin_password = "senhadificil"` appears commented out in
`hosts/desktop/configuration.nix` and `hosts/xpsbipa/configuration.nix`, and
`hosts/xpsbipa/configuration.nix`'s postgres `initialScript` sets
`PASSWORD 'finapp'` for real. The latter is decorative — the same block configures
`host all all 127.0.0.1/32 trust`, so local auth never consults the password — but
it is a literal in a public repo either way.

[agenix]: https://github.com/ryantm/agenix

## CI

Two jobs, and only one of them gates a merge.

| job | required | needs KVM | what it does |
|---|---|---|---|
| `Host evaluation` | **yes** | no | evaluates `notebook`, `xpsbipa`, `precision` toplevels |
| `NixOS VM checks` | no | yes | builds `checks.buildbot-fanout` and boots it |

`desktop` is absent from `Host evaluation` because it imports private
`certus-infra` modules that declare the option surface its own config sets, so a
public runner cannot evaluate it. That is tracked as real work, not an oversight;
it reduces to one secret.

**A red `NixOS VM checks` is usually not a regression.** GitHub's free runners hand
out `/dev/kvm` unreliably: measured across the last 30 runs of this workflow, 6 of
the 26 settled ones ended red at `Enable and verify KVM`, with every step after it
skipped. Check that step first, and if it failed, `gh run rerun <id> --failed`. It
is not a required check precisely because of this.

What eval-only coverage does **not** catch, so you know what a green badge is worth:
build-time check derivations never run, so a bogus `services.postgresql.settings`
key is green until something builds it; an *aliased* nixpkgs rename only warns; a
freeform systemd key typo like `AlowedCPUs` is silently dropped; and a wrong `name`
or hash in a fixed-output derivation evaluates fine. Realising `system.checks` was
tried to close this and reverted — warm it took 9s, cold it compiled Hyprland for 17
minutes and hit the timeout.

### Checks

```
nix build .#checks.x86_64-linux.buildbot-fanout        # public inputs only
nix build .#checks.x86_64-linux.buildbot-workstation   # needs private certus-infra
```

Both boot real VMs and need `/dev/kvm`; each takes about a minute warm.
`buildbot-fanout` proves the CI concurrency lock bounds compilation, using a second
unlocked worker as a control. `buildbot-workstation` boots the desktop's actual
buildbot stack and asserts the capacity limits land on the right cgroup, that the CI
daemon carries them and the system daemon does not, and that the worker
authenticates.

**Never build `buildbot-workstation`, or any host toplevel, in a job that pushes to
a cache.** Its closure carries `certus-infra`'s `writeText` secrets.

## Binary cache

Pull needs nothing: it is armed by the devShell. Push happens from CI on merged
code. If you ever need to push by hand, push only `checks.buildbot-fanout`, and read
`flake.nix` first.

Do not push a check's OUTPUT by hand. A check's output is its verdict, so once it is
in a trusted signed cache `nix build` substitutes it and the test never runs again
for that closure. That happened once here: a hand-pushed `vm-test-run-buildbot-fanout`
took the CI job from 9m20s to 39s with zero subtests executed. The cache has since
been purged, and two belts now prevent a recurrence — CI realises the check with
`--no-substitute`, and `pushFilter` keeps `*-test-run-*` out of the cache — but
there is no `cachix` CLI subcommand for deleting a path, so a mistake here needs the
dashboard to undo.

## desktop and its buildbot

`desktop` runs a buildbot master and worker that build work repositories on pull
requests. The master is reachable on the tailnet at `:8010`, and its GitHub webhook
funnel is published on `:443`. Two local modules shape it:

- `modules/services/buildbot-pr-policy.nix` — a buildbot *configurator*, not a
  patch. It keeps only per-PR schedulers, filters them to listed forge logins, and
  puts a counting lock on every `*/nix-build` builder. Never cap
  `Worker.max_builds` instead: an eval build parks holding a slot until the builds
  it triggered finish, and they need a slot on the same worker, so it deadlocks.
- `modules/services/nix-daemon-ci.nix` — a second, cgroup-limited `nix-daemon` on
  its own socket, sharing the store. It exists because `max-substitution-jobs` is
  trust-gated and therefore unreachable from an untrusted client, and because
  compile limits only bite if they land on the cgroup that actually compiles.

Reading its state, from any machine on the tailnet:

```
curl -s http://desktop:8010/api/v2/schedulers
curl -s 'http://desktop:8010/api/v2/builds?complete=false'
```

Three things that will mislead you here. `nh` reports "Activation (test) failed" on
perfectly good switches, because an unrelated `tempo.service` from the private
observability module fails every activation — judge with `systemctl is-active`, not
`nh`'s exit code. `ping desktop` failing means nothing, because its nftables drops
ICMP. And `bencher` is disabled in `flake.nix` on purpose: its `importCargoLock`
fetches a crate from a legacy crates.io URL that now 403s, which blocks every
desktop rebuild.
