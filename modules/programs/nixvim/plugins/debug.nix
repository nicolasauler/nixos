{
  programs.nixvim = {
    plugins.dap = {
      enable = true;
      extensions.dap-go.enable = true;
      extensions.dap-python.enable = true;
      extensions.dap-ui.enable = true;
      extensions.dap-virtual-text.enable = true;
    };

    extraConfigLua = ''
      local dap, dapui = require("dap"), require("dapui")
      dap.listeners.before.attach.dapui_config = function()
        dapui.open()
      end
      dap.listeners.before.launch.dapui_config = function()
        dapui.open()
      end
      dap.listeners.before.event_terminated.dapui_config = function()
        dapui.close()
      end
      dap.listeners.before.event_exited.dapui_config = function()
        dapui.close()
      end'';

    plugins.rustaceanvim = {
      enable = true;
      settings.tools.hover_actions.replace_builtin_hover = true; # want to test lspsaga's impl
      settings = {
        rust-analyzer = {
          server.extraEnv = {
            CARGO_TARGET_DIR = "target/analyzer";
          };
          check.extraArgs = [
            "--target-dir=target/analyzer"
          ];
          cargo = {
            extraEnv = {
              CARGO_PROFILE_RUST_ANALYZER_INHERITS = "dev";
            };
            extraArgs = [
              "--profile"
              "rust-analyzer"
            ];
          };
          runnables.extraArgs = [
            "--target-dir"
            "target/analyzer"
          ];
        };
        server = {
          server.extraEnv = {
            CARGO_TARGET_DIR = "target/analyzer";
          };
          check.extraArgs = [
            "--target-dir=target/analyzer"
          ];
          cargo = {
            extraEnv = {
              CARGO_PROFILE_RUST_ANALYZER_INHERITS = "dev";
            };
            extraArgs = [
              "--profile"
              "rust-analyzer"
            ];
          };
        };
        # cargo = {
        #   extraEnv = {
        #     CARGO_PROFILE_RUST_ANALYZER_INHERITS = "dev";
        #   };
        #   extraArgs = [
        #     "--profile"
        #     "rust-analyzer"
        #   ];
        # };
      };
    };
  };
}
