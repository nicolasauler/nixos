# Debugging Zig from Neovim

The editor side lives in `modules/programs/nixvim/plugins/debug.nix` and is already
done. This file covers the part that has to be repeated **per Zig project**: what
`build.zig` needs, what the devShell needs, and how to actually drive it.

Everything here was verified against Zig 0.16.0, Neovim 0.12.4, codelldb 1.12.2
(LLDB 21.1.8), gdb 17.2 and rr 5.9.0.

---

## 1. The one thing that matters most

Zig 0.16 compiles Debug x86_64 builds with its **self-hosted backend**, which stamps
`DW_AT_language = 39` (0x27, "Zig") into the DWARF. LLDB has no Zig type system, so
it discards the entire compilation unit's variables: breakpoints still bind, stepping
still works, and **every local is missing**. The LLVM backend stamps `DW_AT_language
= 12` (ANSI C99) and everything resolves.

There is no `zig build -fllvm` — that flag only exists on `zig build-exe`. So
`build.zig` has to expose it as an option.

Check any binary:

```bash
readelf --debug-dump=info --dwarf-depth=1 zig-out/bin/<name> | grep DW_AT_language
#   12 (ANSI C99)  -> debuggable
#   39 (unknown)   -> you will see no variables
```

A plain `zig build` silently reverts the artifact to the undebuggable form. `<leader>dd`
always passes the flag; a manual build or `zig build run` will not.

---

## 2. `build.zig` — three additions

### a. The `use-llvm` option

```zig
// Debug x86_64 builds default to the self-hosted backend, whose DWARF LLDB cannot
// read variables from. Off by default so ordinary builds keep the faster backend.
const use_llvm = b.option(bool, "use-llvm", "Force the LLVM backend (required for debugger locals)");
```

Then thread it through **every artifact you might debug** — the executable and each
test binary:

```zig
const exe = b.addExecutable(.{
    .name = "myproject",
    .root_module = b.createModule(.{ ... }),
    .use_llvm = use_llvm,
});
```

Do **not** also set `.use_lld = use_llvm`; it is unnecessary (verified: `DW_AT_language
= 12` without it).

### b. The `check` step — ZLS diagnostics on save

Semantic analysis only, never reaching the linker, so ZLS gets real compiler
diagnostics on save without paying for codegen or invalidating the `zig build` cache.
This pairs with `enable_build_on_save` / `build_on_save_step = "check"` in the ZLS
config.

```zig
const check_step = b.step("check", "Semantic analysis for ZLS");
check_step.dependOn(&b.addExecutable(.{
    .name = "myproject",
    .root_module = exe.root_module,
}).step);
check_step.dependOn(&b.addTest(.{ .root_module = mod }).step);
check_step.dependOn(&b.addTest(.{ .root_module = exe.root_module }).step);
```

Deliberately **no** `.use_llvm` here — this step never produces a binary, and forcing
LLVM would just slow every save.

### c. The `test-bin` step — debuggable test binaries

`zig build test` runs the tests but leaves their binaries in `.zig-cache` under
content-addressed hashes, so a debugger has nothing stable to point at. Install them
somewhere predictable instead.

Name each test artifact — `addTest` defaults every one of them to `"test"`, so two
would collide:

```zig
const mod_tests = b.addTest(.{
    .name = "mod-test",
    .root_module = mod,
    .use_llvm = use_llvm,
});

const exe_tests = b.addTest(.{
    .name = "exe-test",
    .root_module = exe.root_module,
    .use_llvm = use_llvm,
});
```

Then add the install step (a test binary covers exactly one module, hence two):

```zig
const test_bin_step = b.step("test-bin", "Install test binaries for debugging");
test_bin_step.dependOn(&b.addInstallArtifact(mod_tests, .{
    .dest_dir = .{ .override = .{ .custom = "test" } },
}).step);
test_bin_step.dependOn(&b.addInstallArtifact(exe_tests, .{
    .dest_dir = .{ .override = .{ .custom = "test" } },
}).step);
```

Which gives you `zig-out/test/mod-test` and `zig-out/test/exe-test`.

`modules/programs/nixvim/plugins/debug.nix` has a **"Launch test binary"**
configuration that browses `zig-out/test/`.

> Use `.dest_dir = .{ .override = .{ .custom = "test" } }`, not the older
> `.dest_sub_path = "../test/name"` trick — the latter only works because path
> resolution normalises the `..`.

---

## 3. devShell

`zig` is not installed system-wide by design, so Neovim has to be launched from inside
the project shell or `<leader>dd` fails with
``Zig: no `zig` on PATH — start nvim from the devShell``.

`gdb` does **not** need to be here: the rr configuration pins `${pkgs.gdb}/bin/gdb`
from nixpkgs. Verified — a full rr session passes with none of `zig`, `gdb` or `rr` on
the Neovim process's PATH.

```nix
devShells.default = pkgs.mkShell {
  packages = with pkgs; [
    zig
    zls
    rr # only if you want reverse debugging; you invoke this one yourself
  ];
};
```

---

## 4. Daily use

| key | action |
|---|---|
| `<leader>dd` | build with `-Duse-llvm=true`, then launch (async — the editor stays responsive) |
| `<leader>dc` | continue / start, picking a configuration |
| `<leader>db` / `<leader>dB` | breakpoint / conditional breakpoint |
| `<leader>do` `<leader>di` `<leader>dO` | step over / into / out |
| `<leader>dh` | hover value (works on a visual selection too) |
| `<leader>ds` `<leader>dr` `<leader>du` | scopes / REPL / toggle the UI |
| `<leader>dt` | terminate |
| `<leader>dRc` `<leader>dRo` | reverse continue / reverse step over — **rr sessions only** |

Configurations offered for a `.zig` buffer:

1. **Launch** — `zig-out/bin/<directory name>`
2. **Launch (pick binary)** — prompts inside `zig-out/bin/`
3. **Launch test binary** — prompts inside `zig-out/test/` (needs `zig build test-bin`)
4. **rr replay** — attaches to a running `rr replay` server

Panics stop at the panic handler with your own frame directly below it, rather than
on `SIGABRT` eight frames deep in the abort syscall. Stepping skips `compiler_rt.memcpy`,
which Zig otherwise lowers ordinary struct copies into.

---

## 5. rr — reverse debugging

rr does **not** record for you. Three steps, and the CPU pin is not optional on this
laptop.

```bash
# 1. build with the LLVM backend
zig build -Duse-llvm=true

# 2. RECORD, pinned to a P-core (see below)
rr record --bind-to-cpu="$(cut -d- -f1 </sys/devices/cpu_core/cpus)" -- ./zig-out/bin/myproject

# 3. REPLAY as a debug server, in its own terminal, and leave it running
rr replay -s 50505 -k
```

Then in Neovim: `<leader>dc` → **"rr replay"**.

### Where `--bind-to-cpu` goes, and why

It is an argument to **`rr record`** only — replay re-binds to the recorded CPU
automatically. rr otherwise picks a CPU at random, and this machine (Core Ultra 9
185H) is hybrid: **CPUs 0–11 are P-cores, 12–21 are E-cores**. rr's branch-counting
does not work on an E-core, and you get:

```
[FATAL src/PerfCounters.cc:488:check_working_counters()] Got 0 branch events, expected at least 500
```

Measured: 2 of 3 unpinned runs succeeded, 3 of 3 pinned runs succeeded.

The `$(cut -d- -f1 </sys/devices/cpu_core/cpus)` form reads the first P-core from
sysfs, so it is portable; on a non-hybrid CPU that path does not exist and plain
`--bind-to-cpu=0` is fine.

In **nushell** the substitution differs:

```nu
rr record $"--bind-to-cpu=(open /sys/devices/cpu_core/cpus | split row '-' | first)" -- ./zig-out/bin/myproject
```

If you find yourself typing it often, an alias in `modules/programs/nushell.nix` is
the right home for it.

### Other rr notes

- `-k` (`--keep-listening`) is **mandatory**. Without it the replay server exits on
  the first client disconnect, so every reconnect needs a fresh `rr replay`.
- `rr replay -k` **resumes** where the previous client left off — it does not restart.
  Restart `rr replay` for a clean run.
- **No sysctl change is needed.** `kernel.perf_event_paranoid = 2` is not the blocker;
  the E-core scheduling is. Do not relax it.
- gdb has no Zig pretty-printing, so values are rawer on the rr path than under
  codelldb — `name = 0x7ffc...` rather than `name = "Pedro"`.
- Reverse motions only work on an rr session; fired elsewhere they warn instead of
  silently running forward.

---

## 6. When something looks broken

| symptom | cause | fix |
|---|---|---|
| Variables/Scopes pane empty | self-hosted-backend binary | rebuild with `-Duse-llvm=true`; check `DW_AT_language` |
| `<leader>dd` says `no zig on PATH` | nvim launched outside the devShell | start it from `nix develop` / direnv |
| Breakpoints bind but locals are garbage *before* their declaration line | correct behaviour — the slot is not yet initialised | step past the declaration |
| A `comptime` parameter shows "undeclared identifier" | comptime params have no runtime representation | nothing to do |
| Test binary not found | `zig build test` doesn't install anything | `zig build test-bin -Duse-llvm=true` |
| `rr record` fatals on branch events | landed on an E-core | `--bind-to-cpu` as above |
| Some locals missing even with `-fllvm` | upstream LLVM-backend scope bug, worst around values assigned from nested calls | hoist the call result into a plain local, or step out one frame |

Zig's own LLDB/GDB pretty printers are deliberately **not** loaded: as of 0.16 they
read a `data` member where Zig emits `payload`, so a non-null `?u32` renders as `null`
and every error union as `()`. Plain LLDB is already readable. Re-test after a Zig
upgrade — the re-enable snippet is in the comments of `debug.nix`.
