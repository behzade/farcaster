(function()
  local id, path, line, scratch = unpack(_A)
  local state = rawget(_G, 'farcaster_session_views')
  if not state then
    state = { tabs = {} }
    _G.farcaster_session_views = state
    state.tabs[id] = vim.api.nvim_get_current_tabpage()
  end

  local tab = state.tabs[id]
  if not tab or not vim.api.nvim_tabpage_is_valid(tab) then
    vim.cmd('tabnew')
    tab = vim.api.nvim_get_current_tabpage()
    state.tabs[id] = tab
    if path == nil or path == vim.NIL then
      vim.cmd('edit .')
    end
  else
    vim.api.nvim_set_current_tabpage(tab)
  end

  if scratch ~= nil and scratch ~= vim.NIL then
    local lines = vim.fn.readfile(scratch)
    local buf = vim.api.nvim_create_buf(false, true)
    vim.bo[buf].bufhidden = 'hide'
    vim.bo[buf].swapfile = false
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
    vim.api.nvim_buf_set_name(buf, 'farcaster://transcript/' .. id .. '/' .. buf .. '.md')
    vim.cmd('hide buffer ' .. buf)
    vim.bo[buf].filetype = 'markdown'
    vim.bo[buf].modified = false
    vim.api.nvim_win_set_cursor(0, {1, 0})
  elseif path ~= nil and path ~= vim.NIL then
    vim.cmd('hide edit ' .. vim.fn.fnameescape(path))
    if line ~= nil and line ~= vim.NIL then
      vim.fn.cursor(line, 1)
    end
  end
  return 0
end)()
