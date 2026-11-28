-- The terminal fills partial cells with its default background, independently
-- of Neovim's grid. Match Normal so resizing leaves no contrasting strip.
local previous
local function sync(force)
  local background = vim.api.nvim_get_hl(0, { name = 'Normal', link = false }).bg
  if not force and background == previous then return end
  local sequence = background
      and string.format('\027]11;#%06x\027\\', background)
      or '\027]111\027\\'
  if vim.api.nvim_ui_send then
    vim.api.nvim_ui_send(sequence)
  else
    vim.api.nvim_chan_send(vim.v.stderr, sequence)
  end
  previous = background
end

vim.api.nvim_create_autocmd({ 'VimEnter', 'UIEnter', 'ColorScheme' }, {
  group = vim.api.nvim_create_augroup('FarcasterTerminalBackground', { clear = true }),
  callback = function(event)
    vim.schedule(function() sync(event.event == 'UIEnter') end)
  end,
})
vim.schedule(sync)
