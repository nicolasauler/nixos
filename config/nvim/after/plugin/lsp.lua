require("neodev").setup({
  library = { plugins = { "nvim-dap-ui" }, types = true },
  ...
})

local lsp_zero = require('lsp-zero').preset({})

lsp_zero.on_attach(function(client, bufnr)
    lsp_zero.default_keymaps({ buffer = bufnr })
end)

require('mason').setup({})
require('mason-lspconfig').setup({
    ensure_installed = {
        'arduino_language_server',
        'bashls',
        'clangd',
        'lua_ls',
        --'nil_ls',
        'pyright',
        'rust_analyzer',
        'yamlls',
    },
    handlers = {
        lsp_zero.default_setup,
    },
})

-- (Optional) Configure lua language server for neovim
require('lspconfig').lua_ls.setup(lsp_zero.nvim_lua_ls())

-- config arduino language server
local MY_FQBN = "esp32:esp32:esp32doit-devkit-v1"
require('lspconfig').arduino_language_server.setup {
    cmd = {
        "arduino-language-server",
        "-cli-config",
        "/home/nic/.arduino15/arduino-cli.yaml",
        "-fqbn",
        MY_FQBN,
    }
}

require('lspconfig').nil_ls.setup {
    settings = {
        ['nil'] = {
            testSetting = 42,
            formatting = {
                command = { "nixpkgs-fmt" },
            },
        },
    },
}

require('lspconfig').yamlls.setup {}

lsp_zero.setup()

-- You need to setup `cmp` after lsp-zero
local cmp = require('cmp')
local cmp_action = require('lsp-zero').cmp_action()

require('luasnip.loaders.from_vscode').lazy_load()

cmp.setup({
    sources = {
        { name = 'path' },
        { name = 'nvim_lsp' },
        { name = 'buffer',  keyword_length = 3 },
        { name = 'luasnip', keyword_length = 2 },
    },
    mapping = {
        -- `Enter` key to confirm completion
        ['<C-y>'] = cmp.mapping.confirm({ select = true }),

        -- Ctrl+Space to trigger completion menu
        ['<C-Space>'] = cmp.mapping.complete(),

        --  ['<C-p>'] = cmp.mapping.select_prev_item(cmp_select),
        --  ['<C-n>'] = cmp.mapping.select_next_item(cmp_select),

        -- Navigate between snippet placeholder
        ['<C-f>'] = cmp_action.luasnip_jump_forward(),
        ['<C-b>'] = cmp_action.luasnip_jump_backward(),

        ['<Tab>'] = nil,
        ['<S-Tab>'] = nil,

    }
})

cmp.setup.filetype({ 'sql', 'mysql' }, {
    sources = {
        { name = 'path' },
        { name = 'nvim_lsp' },
        { name = 'buffer',               keyword_length = 3 },
        { name = 'luasnip',              keyword_length = 2 },
        { name = 'vim-dadbod-completion' },
    },
})

lsp_zero.on_attach(function(client, bufnr)
    local opts = { buffer = bufnr, remap = false }

    vim.keymap.set("n", "gd", function() vim.lsp.buf.definition() end, opts)
    vim.keymap.set("n", "<leader>gr", function() vim.lsp.buf.references() end, opts)
    vim.keymap.set("n", "<leader>gi", function() vim.lsp.buf.implementation() end, opts)
    vim.keymap.set("n", "K", function() vim.lsp.buf.hover() end, opts)
    vim.keymap.set("n", "<leader>vws", function() vim.lsp.buf.workspace_symbol() end, opts)
    vim.keymap.set("n", "<leader>vd", function() vim.diagnostic.open_float() end, opts)
    vim.keymap.set("n", "<leader>vh", function() vim.lsp.buf.signature_help() end, opts)
    vim.keymap.set("n", "[d", function() vim.diagnostic.goto_next() end, opts)
    vim.keymap.set("n", "]d", function() vim.diagnostic.goto_prev() end, opts)
    vim.keymap.set("n", "<leader>vca", function() vim.lsp.buf.code_action() end, opts)
    vim.keymap.set("n", "<leader>vrn", function() vim.lsp.buf.rename() end, opts)
end)

vim.diagnostic.config({
    virtual_text = true
})
