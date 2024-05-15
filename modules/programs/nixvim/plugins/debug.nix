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
      tools.hoverActions.replaceBuiltinHover = true; # want to test lspsaga's impl
    };
  };
}
