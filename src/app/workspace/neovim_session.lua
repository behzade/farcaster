(function()
  local id, path, line = unpack(_A)
  local state = rawget(_G, 'farcaster_session_views')
  if not state then
    state = { tabs = {} }
    _G.farcaster_session_views = state
    -- The first session owns the startup tab, including the user's config.
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

  if path ~= nil and path ~= vim.NIL then
    -- Keep modified buffers even when 'hidden' is disabled in user config.
    vim.cmd('hide edit ' .. vim.fn.fnameescape(path))
    if line ~= nil and line ~= vim.NIL then
      vim.fn.cursor(line, 1)
    end
  end
  return 0
end)()
