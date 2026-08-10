{...}: {
  programs.nushell = {
    enable = true;
    shellAliases = {
      builtin-ls = "ls";
      builtin-cd = "cd";
      builtin-ps = "ps";
      cd = "z";
      builtin-cat = "cat";
      cat = "bat";
      vi = "nvim .";
    };
    # The config.nu can be anywhere you want if you like to edit your Nushell with Nu
    # configFile.source = ./.../config.nu;
    # for editing directly to config.nu
    extraConfig = ''
      let carapace_completer = {|spans|
        carapace $spans.0 nushell ...$spans | from json
      }
      $env.config = {
        edit_mode: vi,
        show_banner: false,
        completions: {
          case_sensitive: false # case-sensitive completions
          quick: true    # set to false to prevent auto-selecting completions
          partial: true    # set to false to prevent partial filling of the prompt
          algorithm: "fuzzy"    # prefix or fuzzy
          external: {
            # set to false to prevent nushell looking into $env.PATH to find more suggestions
            enable: true
            # set to lower can improve completion performance at the cost of omitting some options
            max_results: 100
            completer: $carapace_completer # check 'carapace_completer'
          }
        }
      }
      $env.PATH = ($env.PATH |
      split row (char esep) |
      prepend /home/myuser/.apps |
      append /usr/bin/env
      )

      # rr counts retired branches, which only works on P-cores, so an unpinned
      # `rr record` dies on an E-core with "Got 0 branch events" -- intermittently,
      # since it picks one at random. From sysfs rather than hardcoded, to stay right
      # on non-hybrid machines. Needs `rr` on PATH, i.e. the project devShell.
      def rr-pcore [] {
        if ("/sys/devices/cpu_core/cpus" | path exists) {
          open --raw /sys/devices/cpu_core/cpus | str trim | split row "-" | first
        } else { "0" }
      }

      # rr-record ./zig-out/bin/foo        args for the debuggee go after a second --
      def rr-record [...args] {
        rr record $"--bind-to-cpu=(rr-pcore)" -- ...$args
      }

      # Server for nvim-dap's "rr replay" config. -k is mandatory or it exits on the
      # first client disconnect. It resumes where the last client left off; restart for
      # a fresh run.
      def rr-replay [--port (-p): int = 50505] {
        rr replay -s $port -k
      }
    '';
  };

  programs.carapace = {
    enable = true;
    enableNushellIntegration = true;
  };

  programs.eza = {
    enable = true;
    icons = "auto";
    git = true;
    enableBashIntegration = true;
    enableNushellIntegration = true;
  };

  programs.zoxide = {
    enable = true;
    enableBashIntegration = true;
    enableNushellIntegration = true;
  };

  programs.atuin = {
    enable = true;
    enableBashIntegration = true;
    enableNushellIntegration = true;
  };

  programs.fd.enable = true;
}
