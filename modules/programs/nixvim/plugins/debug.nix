{pkgs, ...}: {
  programs.nixvim = {
    plugins.dap.enable = true;
    plugins.dap-go.enable = true;
    plugins.dap-python.enable = true;
    plugins.dap-ui.enable = true;
    plugins.dap-virtual-text.enable = true;

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

    keymaps = [
      {
        key = "<leader>do";
        action.__raw = "function() require('dap').step_over() end";
        mode = "n";
        options.desc = "[D]ap Step-[O]ver";
      }
      {
        key = "<leader>di";
        action.__raw = "function() require('dap').step_into() end";
        mode = "n";
        options.desc = "[D]ap Step-[I]nto";
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
