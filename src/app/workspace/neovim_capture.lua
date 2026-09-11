(function()
  local mode = vim.fn.mode()
  if mode ~= 'n' and mode ~= 'v' and mode ~= 'V' and mode ~= '\22' then
    error('Use normal or visual mode to send to chat')
  end
  local path = vim.api.nvim_buf_get_name(0)
  if path == '' or vim.bo.buftype ~= '' then
    error('Open a file buffer to send to chat')
  end
  local cursor = vim.fn.getpos('.')
  local anchor = mode == 'n' and cursor or vim.fn.getpos('v')
  if math.abs(cursor[2] - anchor[2]) > 1999 then
    error('Select at most 2,000 lines to send to chat')
  end
  local lines
  if mode == 'n' then
    lines = { vim.api.nvim_get_current_line() }
  else
    lines = vim.fn.getregion(anchor, cursor, {
      type = mode,
      exclusive = vim.o.selection == 'exclusive',
    })
  end
  local text = table.concat(lines, '\n')
  if #text > 131072 then
    error('Select at most 128 KiB of code to send to chat')
  end
  return vim.json.encode({
    path = path,
    cursor_line = cursor[2],
    cursor_column = cursor[3],
    anchor_line = anchor[2],
    anchor_column = anchor[3],
    mode = mode,
    text = text,
    modified = vim.bo.modified,
  })
end)()
