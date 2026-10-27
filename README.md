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

## precision microphone keys

`talkd` handles the Voyager's **F13**: hold to invert the default microphone's
mute, release to restore, or tap under 250 ms to latch until the next press.
**F14** is a separate, ordinary default-input mute toggle through `wpctl`; it
does not send a command to talkd or change the speaker output.

Use F14 before starting an F13 hold/latch cycle to establish the baseline:
muted gives hold-to-talk; unmuted gives hold-to-mute. Each F14 press uses the
current default input, including a newly selected headset. F14 is disabled
while the screen is locked; an existing F13 cycle can still finish.

The accepted Oryx revision **JqpmQ/EeKlbD** assigns F13 to base top-right
(formerly Caps Word). F14 is **hold Enter, press AltGr**; the owner chose this
instead of Tab+top-right, which falls through to F13. The symbol layer also has
F13 on minus and F15–F17 on slash/period/comma; F1–F12 are unchanged.
Changing or switching this Nix configuration does not flash the keyboard.
`talkd status` inspects the audio/lock setup without changing it.

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

## Desktop status bar

All four desktop hosts share `modules/programs/waybar/waybar.nix` and its
Gruvbox stylesheet; laptop homes append the battery module. The package is
stock nixpkgs Waybar, without the redundant experimental build override.

Workspaces and the focused window's title sit on the left. Everything else is
right-aligned on one line: the hardware block (CPU, per-CPU strip, RAM, disk and
precision's sensors), then microphone, output volume, network, clock, tray and,
on laptops, battery. Text is bold 15px InconsolataGo Nerd Font Mono (Alacritty's
family, registered with fontconfig on every host): blue labels, white values
and a purple clock, date and time alike, in a 24px bar. Network text is capped
at 26 characters and the click-to-toggle date is short. With those longest
strings, ten workspaces and two tray icons, precision's right-aligned modules
need about 1640px, so its 1920px and 2160px screens fit: the window title and
network name shorten with `…` as needed before anything would clip. The 1080px
portrait monitor that desktop and notebook configure still cuts off the right
side. Nothing hides behind hover drawers:

- CPU: overall utilization, **load** = 1-minute load average (tasks running,
  waiting for a CPU or blocked on disk; not utilization and not divided by the
  CPU count, so compare it with the 22 logical CPUs), and average reported
  GHz. These help distinguish a busy build, a serial phase and reduced clocks.
  The overall figure is native Waybar's, which adds guest (VM) time on top of
  the user/nice time that already contains it: while virtual machines run, it
  can read higher than the per-CPU strip.
- Per-logical-CPU usage: one `▁`–`█` bar per observed CPU, all on one line;
  the tooltip gives each bar's actual `cpuN` and exact percentage. More CPUs
  lengthen the strip, never the bar's height. `waybar-cores` samples
  `/proc/stat` every two seconds, handles hotplug by ID, excludes guest
  double-counting and idle/iowait, and shows `?` for an invalid or
  unestablished delta rather than fake idle time. It is a std-only Rust Waybar
  child, not a service or per-update shell command.
- RAM: used/total GiB continuously; available RAM and swap remain in the
  tooltip. Click CPU, the strip or RAM for htop.
- Root filesystem: used percentage and available bytes continuously. The
  85%/95% thresholds change color, not visibility.
- On precision: the identified coretemp package sensor and NVMe controller 0
  composite temperature. Their stable device paths resolve the changing
  `hwmonN` child at startup/reload. A path missing at startup/reload disables
  that widget rather than falling back to an unrelated ACPI zone. If an
  already-selected path disappears later, the native widget can retain its
  last reading until reload. Other hosts' sensor paths are not guessed.
  UI warning/critical thresholds are 90/100°C for CPU and 70/80°C for NVMe,
  not claims about hardware shutdown limits.

Network shows the interface and the Wi-Fi network, or the interface and `wired`;
both addresses stay in its tooltip. With both connected, it shows the one
carrying the default route (normally the wired link). The clock reads
day/month; hovering shows the whole year with today highlighted, a click
toggles the full date, right click switches between year and month, and
scrolling moves through them. Laptop
battery warnings start at 20% and become critical at 10%. Battery estimates
remain in its tooltip. There is no GPU polling,
fan/SMM probing, per-module rainbow or blinking. Reload Waybar if a driver
re-enumerates its sensor paths.

The microphone indicator is read-only:

- **MIC MUTED**: the current default input is muted.
- **MIC LIVE**: that input is unmuted, not proof that an application is recording.
- **MIC ?**: the default input or its mute state is unavailable/synchronizing.

`talkd waybar` supplies newline-delimited JSON using the same PipeWire
Route/Props reader as F13. It starts unknown, updates on events, and reconnects
after server loss without retaining a stale muted label. It never opens input
devices, changes mute/volume, queries the compositor, writes restoration state
or sends notifications. It runs only as a Waybar child, not another systemd
service. Device text is escaped by Waybar, and clicking/scrolling this indicator
does nothing.

The bar follows the **default input**, not an active talkd cycle. If the default
changes during an F13 hold/latch, that cycle stays on its original microphone.
On precision, F14 remains the explicit baseline toggle before starting a cycle;
the other hosts get the indicator without enabling talkd's keyboard controls.

Output volume uses the native WirePlumber module: scroll adjusts it, click opens
pavucontrol. Unlike the microphone stream, the native output widget does not
reconnect after a PipeWire restart; reload Waybar then. Also reload after
installing a changed bar configuration (Hyprland owns its launch):

```nu
pkill --signal USR2 waybar
```

This reloads the bar; `Super+B` only hides/shows it. Waybar itself hosts the
tray's StatusNotifierWatcher, and both this reload and a restart replace it:
apps that do not register again (Slack and 1Password here; nm-applet does)
lose their tray icon until they are restarted.

## precision keyboard reference

Open **Super+D**, search **Voyager Keyboard Reference**, then search a function,
key or alias such as `caps`, `F11`, `click`, `underscore` or `mirror`.
The Rofi view shows the physical hold/tap recipe and layer. Escape closes it;
Enter closes it when a result is selected. Neither executes an action, and
accepting a row emits no row text. Auto-select is disabled so a single match
stays visible. Symbol searches, including a literal `-`, are not negation filters.

`voyager-help` reads the connected Voyager's USB **layout/revision and product**
metadata, then generates the reference from that exact Oryx revision—not the
latest saved layout. It reads no input events or raw-HID devices and runs only
when opened. There is no hand-maintained list of user key positions.

The first lookup of a revision needs network access. Its validated Oryx response
is cached in `$XDG_CACHE_HOME/voyager-help/` (normally `~/.cache/voyager-help/`);
subsequent lookups of that revision work offline. A changed revision selects its
own cache. Missing, malformed or ambiguous keyboard identities, or an unavailable
exact revision, produce an error rather than silently showing an older map.

Recipes start from base with no held keys or latches. The generator follows
momentary/move/toggle/default layer actions, whole-key precedence and held-key
collisions. It describes tap/hold/double-tap/tap-hold actions and derives Shift
symbols for the configured `us / altgr-intl` host layout. Coordinates identify
physical positions even when base labels are repeated. Unknown actions/macros
remain explicit; the bounded route search reports unresolved routes, not guesses.
Friendly action names, search aliases and precision's F13/F14 host meanings live
in the helper, but ordinary key positions and layer paths come from Oryx.
Oryx custom labels such as “Prev Tab” are searchable aliases, not assumed
executable semantics.

Custom builds from `firmware/voyager` keep the Oryx serial and add
`Voyager nic-swap-v1:<source fingerprint>` as the USB product. The same Nix
derivation produces a reference profile, embedded in the matching helper.
It binds the layout/revision, source pins and bytes, layer parameters and
Voyager geometry. Selecting profile metadata does not build the firmware.

For a recognized custom build, the helper applies Swap Hands before layer
lookup and shows normal and mirrored recipes. It tracks physical holds and
their mapped press positions to avoid recommending QMK cache collisions.
Mirror/nav pedal routes require the matching daemon. Releasing only mirror
preserves unrelated keyboard layers; ambiguous combined-pedal release paths
are not used when separate/coalesced releases could produce different layers.

Stock `Voyager` product metadata keeps the ordinary Oryx interpretation. Unknown
custom products or mismatched serial/profile data are refused rather than
silently interpreted as stock. **This is source identification, not hardware
attestation:** a device can copy descriptors, and an unmarked custom build can
still look stock. Arbitrary new C behavior also needs an updated semantic model.

## precision under heavy builds

Several Rust builds at once (agents each running bipa's `dev clippy`, `dev up` or
`dev test`, at about 20 GiB apiece) used to fill all 62 GiB and freeze the desktop.
Two limits now keep RAM for it:

- Nix builds share one cap: `nix-daemon.service` may use 50 GiB, which leaves about
  12 GiB for the desktop with Slack and Firefox open. At the cap the kernel OOM-kills
  processes inside it, usually a compiler, instead of swapping. systemd does not
  stop the rest of the unit on top of that, but which build loses is the kernel's
  choice.
- `dev test` compiles with cargo in the agent's own shell, outside that cap, so
  earlyoom backs it up. When available memory falls below about 3 GiB it kills the
  largest process by memory, system-wide: compilers, linkers and cargo count 3 GiB
  bigger, Firefox, Slack and the agents 3 GiB smaller. These are biases, not tiers:
  other work can be killed even while builds run. Hyprland, Xwayland, Waybar,
  alacritty and zellij themselves are excluded. `journalctl -u earlyoom` names each
  kill. earlyoom 1.9.0 needs upstream's fix for `--avoid` under `--sort-by-rss`, so
  precision builds it with that patch. Once nixpkgs ships a newer earlyoom,
  evaluation fails until the patch is dropped and the sizes above rechecked.

A killed build fails and needs a rerun. Both limits apply on switch, no reboot
needed.

## SSH between desktop and precision

Each machine takes SSH from the other over the tailnet, and from nothing else.

- **desktop → precision:** `ssh precision`. precision's sshd accepts only `nic`,
  only with desktop's key (its `~/.ssh/id_ed25519.pub`, the one it signs commits
  with), and only from desktop's two tailnet addresses: no passwords, no root.
  Port 22 is open on `tailscale0` only. desktop pins precision's host key, so the
  first `ssh precision` doesn't ask to trust it.
- **precision → desktop:** desktop's nftables rule accepts port 22 from
  precision's tailnet address only, and you log in with desktop's password.

On precision the restriction lives in sshd (`AllowUsers`), not in the firewall:
tailscaled runs there in iptables mode and accepts everything arriving on
`tailscale0` before NixOS's firewall sees it, so anything precision listens on is
reachable from the whole tailnet. The addresses and keys are written into both
host configs. If desktop gets a new key, or either machine re-registers with
Tailscale and gets new addresses (`tailscale status`), update
`hosts/precision/configuration.nix` (`AllowUsers`, the authorized key) and
`hosts/desktop/configuration.nix` (the nftables rule, the pinned host key).

## Voyager firmware build and flash

Oryx remains the editor. Save and compile there, then from the firmware checkout:

```sh
nix run .#voyager-layout-update
nix build .#voyager-firmware --out-link result-voyager
```

Review the fetched revision before flashing. Layer 6 must be entirely
transparent; it activates the custom Swap Hands module. Layer 7 is an alias,
not an Oryx layer: mirror plus navigation pedals request 7, which the hook
expands to navigation 4 plus mirror 6. Mouse 5 is not combined by the pedals.
The accepted layout puts navigation Shift on Tab and GUI/Ctrl/Alt on A/R/S,
keeps mouse 5 unchanged, and uses equals/quote as keyboard mirror holds.

After review, outside a call with all keys/pedals released:

```sh
systemctl --user stop foot-pedal talkd
env NIXPKGS_ALLOW_UNFREE=1 nix run --impure nixpkgs#zapp -- flash result-voyager/*-swap.bin
systemctl --user start foot-pedal talkd
```

Press the physical reset button when Zapp requests bootloader mode; wait for
success before the last command. Use the local custom binary, not an Oryx URL
or `zapp update`, which would install stock firmware. Keep the known-good stock
binary and laptop keyboard available for recovery.

**Flash before activating the alias-7 daemon.** Verify ordinary keys, mirror
holds and F13/F14 first; merge only after the owner's explicit go, then rebuild
and test mirror+nav together. Until that switch, use the matching preview
`voyager-help` package for custom-firmware lookup, not the older installed helper.
Rolling back NixOS does not reflash the keyboard; restoring stock firmware also
requires stopping or rolling back the alias-7 daemon.

Accepted QMK boundaries remain: wait for a layer-tap mirror hold to resolve
(about 200 ms) before typing; keyboard/pedal layer bits are shared; do not hold
a key through a mirror transition and then press its counterpart. For a stuck
modifier, return to unmirrored base, hold its owning mod-tap past the tapping
term, then release it.

## CI

Two jobs, and only one of them gates a merge.

| job | required | needs KVM | what it does |
|---|---|---|---|
| `Host evaluation` | **yes** | no | evaluates `notebook`, `xpsbipa`, `precision` toplevels |
| `NixOS VM checks` | no | yes | builds and boots `checks.buildbot-fanout`, `checks.nix-substitution-limit`, `checks.fingerprint-pam`, `checks.printing-on-demand` |

`desktop` is absent from `Host evaluation` because it imports private
`certus-infra` modules that declare the option surface its own config sets, so a
public runner cannot evaluate it. That is tracked as real work, not an oversight;
it reduces to one secret.

**A red `NixOS VM checks` is usually not a regression.** GitHub's free runners hand
out `/dev/kvm` unreliably: of the 51 settled runs of this workflow since
2026-08-31, 9 ended red at `Enable and verify KVM` — and every red run in that
window ended there, with every step after it skipped. Check that step first, and
if it failed, `gh run rerun <id> --failed`. It is not a required check precisely
because of this.

What eval-only coverage does **not** catch, so you know what a green badge is worth:
build-time check derivations never run, so a bogus `services.postgresql.settings`
key is green until something builds it; an *aliased* nixpkgs rename only warns; a
freeform systemd key typo like `AlowedCPUs` is silently dropped; and a wrong `name`
or hash in a fixed-output derivation evaluates fine. Realising `system.checks` was
tried to close this and reverted — warm it took 9s, cold it compiled Hyprland for 17
minutes and hit the timeout.

### Checks

```
nix build .#checks.x86_64-linux.buildbot-fanout          # public inputs only
nix build .#checks.x86_64-linux.nix-substitution-limit   # public inputs only
nix build .#checks.x86_64-linux.fingerprint-pam          # public inputs only
nix build .#checks.x86_64-linux.printing-on-demand       # public inputs only
nix build .#checks.x86_64-linux.buildbot-workstation     # needs private certus-infra
```

All five boot real VMs and need `/dev/kvm`. Each takes about a minute warm, except
`printing-on-demand`, which waits out cupsd's idle timeout and takes nearly three.
`buildbot-fanout` proves the CI concurrency lock bounds compilation, using a
second unlocked worker as a control. `nix-substitution-limit` proves the CI daemon's
`max-substitution-jobs` bounds concurrent NAR fetches, with the system daemon as
the control. `fingerprint-pam` proves, against libfprint's virtual reader, that
`polkit-1` accepts a fingerprint on its own and is the only PAM service that
consults the reader. `printing-on-demand` proves, against CUPS's own IPP Everywhere
simulator, that a printer announced over mDNS prints for an ordinary user with no
driver and no queue, that no other host can reach CUPS or wake it, and that cupsd
starts only when something prints and exits by itself afterwards, with the module's
on-demand settings reverted as the control.
`buildbot-workstation` boots the desktop's actual buildbot stack and asserts the
capacity limits land on the right cgroup, that the CI daemon carries them and the
system daemon does not, and that the worker authenticates.

**Never build `buildbot-workstation`, or any host toplevel, in a job that pushes to
a cache.** Its closure carries `certus-infra`'s `writeText` secrets.

## Binary cache

Pull needs nothing: it is armed by the devShell. Push happens from CI on merged
code. If you ever need to push by hand, push only the four public checks
(`buildbot-fanout`, `nix-substitution-limit`, `fingerprint-pam`,
`printing-on-demand`), and read `flake.nix` first.

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
