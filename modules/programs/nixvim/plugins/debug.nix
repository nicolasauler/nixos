{pkgs, ...}: let
  # Shared with rustaceanvim, whose adapter is private to it.
  codelldb = "${pkgs.vscode-extensions.vadimcn.vscode-lldb}/share/vscode/extensions/vadimcn.vscode-lldb/adapter/codelldb";

  # Build requirements and DWARF verification: see docs/zig-debugging.md.
  mkZigLaunch = name: program: {
    inherit name program;
    type = "codelldb";
    request = "launch";
    cwd = "\${workspaceFolder}";

    # Zig's own formatters (in "${pkgs.srcOnly pkgs.zig}/tools/", absent from the built
    # output) are deliberately not loaded: as of 0.16 they read a `data` member where
    # Zig emits `payload`, so a non-null ?u32 renders as `null` and error unions as
    # `()`. Same bug in the gdb copy. Re-test after a zig bump, then prepend
    # "command script import <that path>/lldb_pretty_printers.py" and
    # "type category enable zig.lang" / "zig.std".
    #
    # Do not add `follow-fork-mode child`: it follows the fork+exec std.process.Child
    # performs, leaving the launched parent undebugged.
    #
    # Zig lowers struct copies to compiler_rt.memcpy: 16 of 20 step-intos land there
    # without this, 0 of 20 with it. Do not add `start.` -- it also matches a user
    # src/start.zig and blocks stepping from stopOnEntry into main.
    initCommands = ["settings set target.process.thread.step-avoid-regexp ^compiler_rt[.]"];

    # preRunCommands, not initCommands: the target must exist first. Turns a panic
    # stopping on SIGABRT eight frames deep in abort into a stop at the panic handler
    # with your own frame directly below. Unmatched names are simply pending.
    preRunCommands = [''breakpoint set --name "debug.defaultPanic"''];

    # Route output into dap-view; unlike "integrated" this has no stdin.
    terminal = "console";
    # Make REPL input an expression; LLDB commands take a backtick prefix.
    _adapterSettings.consoleMode = "evaluate";
  };
in {
  programs.nixvim = {
    plugins.dap.enable = true;

    # Uncomment when needed; dap-python also drags debugpy into the closure.
    # plugins.dap-go.enable = true;
    # plugins.dap-python.enable = true;

    # Superseded by dap-view; explicit `false` so reverting is one word.
    plugins.dap-ui.enable = false;
    plugins.dap-virtual-text.enable = false;

    plugins.dap-view = {
      enable = true;
      settings = {
        auto_toggle = true;

        winbar = {
          controls.enabled = true;
          # Defaults plus "disassembly": register_view() only fills custom_sections,
          # so without naming it here the view is unreachable.
          sections = [
            "watches"
            "scopes"
            "exceptions"
            "breakpoints"
            "threads"
            "repl"
            "disassembly"
          ];
        };

        # Live only for adapters answering runInTerminal. The adapter-hiding key
        # beside it is `hide`; `start_hidden` does not exist and dap-view hard-errors
        # on unknown keys, which nixvim's freeform settings will not catch.
        windows.terminal.position = "right";

        # Stands in for nvim-dap-virtual-text; off by default.
        virtual_text.enabled = true;
      };
    };

    # Comptime folds a lot away; disassembly is the only place to confirm what survived.
    plugins.dap-disasm = {
      enable = true;
      # dapview_register is gated on `package.loaded["dap-view"]`, and nixvim's import
      # order is lexicographic, so force the load or lose the section and gain a
      # second winbar.
      luaConfig.pre = ''require("dap-view")'';
    };

    # Its F-key defaults bind nothing (setup() reads `opts.mappings or {}` and nixvim
    # emits setup({})), so <leader>dR* below are the only reverse motions. If you do
    # set settings.mappings, use F-keys only: teardown restores via nvim_get_keymap,
    # which never reports <leader> maps, so those would be deleted permanently.
    plugins.dap-rr.enable = true;

    # dap-rr claims both filetypes with only its rr entry, so C++ would dial a dead
    # gdbserver and rr would outrank every rustaceanvim debuggable. Nixvim elides these.
    plugins.dap.configurations.cpp = [];
    plugins.dap.configurations.rust = [];

    # Nixvim emits its setup() before its own wholesale dap.adapters/configurations
    # assignments, so everything it registers is destroyed two statements later.
    plugins.dap-lldb.enable = false;

    plugins.dap.adapters.servers.codelldb = {
      port = "\${port}";
      executable = {
        command = codelldb;
        args = ["--port" "\${port}"];
      };
    };

    # Drives gdb for nvim-dap-rr; LLDB cannot reverse-execute, so this is what makes rr
    # possible. `id` is required or nvim-dap sends adapterID "nvim-dap" and
    # OpenDebugAD7 exits without answering.
    plugins.dap.adapters.executables.cppdbg = {
      id = "cppdbg";
      command = "${pkgs.vscode-extensions.ms-vscode.cpptools}/share/vscode/extensions/ms-vscode.cpptools/debugAdapters/bin/OpenDebugAD7";
    };

    plugins.dap.configurations.zig = [
      # `zig init` names the artifact after the directory, so this needs no Lua.
      (mkZigLaunch "Launch" "\${workspaceFolder}/zig-out/bin/\${workspaceFolderBasename}")
      (mkZigLaunch "Launch (pick binary)" {
        __raw = ''
          function()
            return vim.fn.input("Executable: ", vim.fn.getcwd() .. "/zig-out/bin/", "file")
          end
        '';
      })
      # Needs `zig build test-bin -Duse-llvm=true`: plain `zig build test` leaves its
      # binaries in .zig-cache under hashes with nothing stable to point at.
      (mkZigLaunch "Launch test binary" {
        __raw = ''
          function()
            return vim.fn.input("Test binary: ", vim.fn.getcwd() .. "/zig-out/test/", "file")
          end
        '';
      })
      # `__rr` marks the genuine rr config: a launch.json entry could reuse both the
      # cppdbg adapter and this name. `program` skips the telescope picker (a replay
      # must use the recorded binary), gdb is pinned because it is not on PATH, and
      # externalConsole would request a terminal that is not configured.
      #
      # rr does not record for you: `rr record --bind-to-cpu=0 ./zig-out/bin/<name>`,
      # then `rr replay -s 50505 -k`, and only then pick this.
      {
        __raw = ''
          require('nvim-dap-rr').get_config({
            name = "rr replay",
            __rr = true,
            program = "''${workspaceFolder}/zig-out/bin/''${workspaceFolderBasename}",
            miDebuggerPath = "${pkgs.gdb}/bin/gdb",
            externalConsole = false,
          })
        '';
      }
    ];

    extraConfigLua = ''
      local dap = require("dap")

      -- codelldb puts ~275 std internals under "Static"; dap-view has no setting for it.
      require("dap-view.state").collapsed_scopes = { "Static" }

      local function is_rr(session)
        return session and session.config and session.config.__rr == true
      end

      -- gdb's exec-direction is session-wide, so after a reverse motion every later
      -- step runs backwards. Resetting on each stop also covers dap-view's clickable
      -- controls, which a per-keymap wrapper would miss. current_frame suppresses a
      -- failing evaluate on the very first rr stop.
      dap.listeners.after.event_stopped.rr_forward = function(session)
        if is_rr(session) and session.current_frame then
          session:evaluate("-exec set exec-direction forward")
        end
      end

      function _G.DapRev(action)
        if not is_rr(dap.session()) then
          vim.notify("Reverse execution needs an rr session", vim.log.levels.WARN)
          return
        end
        require("nvim-dap-rr")[action]()
      end

      -- Build first: nvim-dap launches a stale binary without complaint. Deliberately
      -- no fallback to plain `zig build`, which yields zero inspectable locals.
      local zig_building = false

      local function zig_quickfix(root, text)
        -- %D anchors zig's root-relative diagnostics: the root comes from walking up
        -- from the buffer, so it need not match the cwd quickfix resolves against.
        local lines = vim.split(text, "\n")
        table.insert(lines, 1, "ZIG_BUILD_ROOT=" .. root)
        local items = vim.fn.getqflist({
          lines = lines,
          efm = "%DZIG_BUILD_ROOT=%f,%f:%l:%c: %t%*[^:]: %m",
        }).items
        table.remove(items, 1)
        -- An invalid *first* row makes :cfirst dead-end. When nothing parses at all
        -- (e.g. "invalid option: -Duse-llvm") keep every row and use copen, since
        -- cwindow refuses to open a list with no valid entries.
        local valid = vim.tbl_filter(function(item)
          return item.valid == 1
        end, items)
        vim.fn.setqflist({}, " ", {
          title = "zig build",
          items = #valid > 0 and valid or items,
        })
        vim.cmd(#valid > 0 and "botright cwindow 15" or "botright copen 15")
      end

      function _G.ZigBuildAndDebug()
        if zig_building then
          vim.notify("Zig: build already in progress", vim.log.levels.WARN)
          return
        end
        -- Buffer's project, then cwd: vim.fs.root(0, ...) is nil for a buffer outside one.
        local root = vim.fs.root(0, "build.zig") or vim.fs.root(vim.fn.getcwd(), "build.zig")
        if not root then
          vim.notify("Zig: no build.zig found from this buffer or cwd", vim.log.levels.WARN)
          return
        end

        if vim.fn.executable("zig") == 0 then
          vim.notify("Zig: no `zig` on PATH — start nvim from the devShell", vim.log.levels.ERROR)
          return
        end
        vim.notify("Zig: building...", vim.log.levels.INFO)
        zig_building = true
        -- Async on purpose: vim.system():wait() is fast_only, so it freezes the editor
        -- for the whole build and starves the notify above (193 callbacks queued, 0 run).
        local ok, err = pcall(
          vim.system,
          { "zig", "build", "-Duse-llvm=true" },
          { cwd = root, text = true },
          vim.schedule_wrap(function(out)
            zig_building = false
            -- A signalled build reports code 0; checking only `code` runs the stale binary.
            if out.code ~= 0 or out.signal ~= 0 then
              local errors = out.stderr or ""
              if errors == "" then
                errors = ("zig build exited %d (signal %d)"):format(out.code, out.signal)
              end
              vim.notify(errors, vim.log.levels.ERROR)
              zig_quickfix(root, errors)
              return
            end
            -- Auto-select a lone executable, else prompt: trusting the directory
            -- basename would launch a stale binary from a differently-named build.
            local bin_dir = root .. "/zig-out/bin/"
            local programs = vim.tbl_filter(function(path)
              return vim.fn.executable(path) == 1
            end, vim.fn.glob(bin_dir .. "*", false, true))
            local program = #programs == 1 and programs[1] or vim.fn.input("Executable: ", bin_dir, "file")
            if program == "" then
              return
            end
            -- Explicit config, resolved by name: dap.continue() would resolve
            -- ''${workspaceFolder} to cwd rather than the root just built and does nothing
            -- from a non-zig buffer, and an index would break if an entry were inserted.
            local base
            for _, c in ipairs(dap.configurations.zig) do
              if c.name == "Launch" then
                base = c
              end
            end
            if not base then
              vim.notify("Zig: no \"Launch\" configuration found", vim.log.levels.ERROR)
              return
            end
            dap.run(vim.tbl_extend("force", base, { cwd = root, program = program }))
          end)
        )
        -- Spawn can still throw after the checks (vanished cwd); release the guard.
        if not ok then
          zig_building = false
          vim.notify("Zig: failed to start build: " .. tostring(err), vim.log.levels.ERROR)
        end
      end
    '';

    # Suffix, don't prefix, rust-analyzer onto the wrapper's PATH so a project
    # devshell's toolchain-paired RA wins while non-nix projects still get one
    # (verified via headless probe: as a prefix the wrapper's RA 2026-06-15
    # shadowed bipa's nightly-2026-07-27). rustaceanvim's default server.cmd
    # already resolves rust-analyzer through $PATH, so this is the only knob.
    dependencies.rust-analyzer.packageFallback = true;

    plugins.rustaceanvim = {
      enable = true;
      settings.tools.hover_actions.replace_builtin_hover = true; # want to test lspsaga's impl
      settings = {
        dap = {
          adapter = {
            type = "server";
            name = "codelldb";
            port = "\${port}";
            executable = {
              command = codelldb;
              args = ["--port" "\${port}"];
            };
          };

          autoload_configurations = true;
        };
        server = {
          on_attach = ''
            function(_, bufnr)
              vim.keymap.set("n", "<leader>rd", function()
                vim.cmd.RustLsp("debuggables")
              end, { desc = "Rust Debuggables", buffer = bufnr })
            end
          '';
          default_settings = {
            rust-analyzer = {
              checkOnSave = false;
              check = {
                command = "check";
                allTargets = true;
                features = "all";
              };
              inlayHints = {
                lifetimeElisionHints = {
                  enable = "skip_trivial";
                };

                # show intermediate types in method chains
                chainingHints.enable = true;

                # parameter names at call sites
                parameterHints.enable = false;

                typeHints = {
                  enable = false; # you mostly know your types; hover when you don't
                  hideClosureInitialization = true; # hide when obvious from RHS
                  hideNamedConstructor = true; # hide Vec::new() -> Vec<T>
                };

                # closure return types only on block bodies
                closureReturnTypeHints.enable = "with_block";

                # implicit ref/ref mut in pattern matching
                bindingModeHints.enable = true;

                # everything below is off — too noisy for daily use
                closureCaptureHints.enable = false;
                expressionAdjustmentHints.enable = "never";
                discriminantHints.enable = "never";
                rangeExclusiveHints.enable = false;
              };
            };
          };
          standalone = false;
        };
      };
    };

    keymaps = [
      {
        key = "<leader>dc";
        action.__raw = "require('dap').continue";
        mode = "n";
        options.desc = "[D]ap [C]ontinue / start";
      }
      {
        key = "<leader>dd";
        action.__raw = "function() _G.ZigBuildAndDebug() end";
        mode = "n";
        options.desc = "[D]ap [D]ebug: zig build, then launch";
      }
      {
        key = "<leader>db";
        action.__raw = "require('dap').toggle_breakpoint";
        mode = "n";
        options.desc = "[D]ap Toggle [B]reakpoint";
      }
      {
        key = "<leader>dB";
        action.__raw = ''
          function()
            require('dap').set_breakpoint(vim.fn.input("Breakpoint condition: "))
          end
        '';
        mode = "n";
        options.desc = "[D]ap Conditional [B]reakpoint";
      }
      {
        key = "<leader>dt";
        action.__raw = "require('dap').terminate";
        mode = "n";
        options.desc = "[D]ap [T]erminate";
      }
      {
        key = "<leader>du";
        action = "<cmd>DapViewToggle<cr>";
        mode = "n";
        options.desc = "[D]ap Toggle [U]I";
      }
      {
        key = "<leader>dr";
        # dap-view owns a REPL section; dap.repl.toggle() would open a *second*,
        # unmanaged REPL window next to it.
        action = "<cmd>DapViewJump repl<cr>";
        mode = "n";
        options.desc = "[D]ap [R]EPL";
      }
      {
        key = "<leader>do";
        action.__raw = "require('dap').step_over";
        mode = "n";
        options.desc = "[D]ap Step-[O]ver";
      }
      {
        key = "<leader>di";
        action.__raw = "require('dap').step_into";
        mode = "n";
        options.desc = "[D]ap Step-[I]nto";
      }
      {
        key = "<leader>dO";
        action.__raw = "require('dap').step_out";
        mode = "n";
        options.desc = "[D]ap Step-[O]ut";
      }
      {
        key = "<leader>dh";
        action.__raw = "require('dap.ui.widgets').hover";
        mode = ["n" "v"];
        options.desc = "[D]ap [H]over value (uses the visual selection in v)";
      }
      {
        key = "<leader>ds";
        action = "<cmd>DapViewJump scopes<cr>";
        mode = "n";
        options.desc = "[D]ap [S]copes";
      }
      {
        # rr only. These go through nvim-dap-rr rather than dap.reverse_continue()
        # / dap.step_back(): cppdbg does not implement the DAP reverse requests, so
        # the plugin flips gdb's exec-direction and then steps forward instead.
        key = "<leader>dRc";
        action.__raw = "function() _G.DapRev('reverse_continue') end";
        mode = "n";
        options.desc = "[D]ap [R]everse [C]ontinue (rr)";
      }
      {
        key = "<leader>dRo";
        action.__raw = "function() _G.DapRev('reverse_step_over') end";
        mode = "n";
        options.desc = "[D]ap [R]everse Step-[O]ver (rr)";
      }
      {
        key = "<leader>le";
        action = "<cmd>RustLsp explainError<CR>";
        mode = "n";
        options.desc = "[L]sp Explain [E]rror";
      }
      {
        key = "<leader>od";
        action = "<cmd>RustLsp openDocs<CR>";
        mode = "n";
        options.desc = "[O]pen docs.rs [D]oc for symbol under cursor";
      }
      {
        key = "<leader>lr";
        action = "<cmd>RustLsp renderDiagnostic<CR>";
        mode = "n";
        options.desc = "[L]sp [R]ender diagnostic: a bit more verbose than line diagnostic";
      }
    ];
  };
}
