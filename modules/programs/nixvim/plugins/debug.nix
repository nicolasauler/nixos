{pkgs, ...}: {
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
        dap = {
          adapter = {
            type = "server";
            name = "codelldb";
            port = "\${port}";
            executable = {
              command = "${pkgs.vscode-extensions.vadimcn.vscode-lldb}/share/vscode/extensions/vadimcn.vscode-lldb/adapter/codelldb";
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
        };
      };
    };
  };
}
