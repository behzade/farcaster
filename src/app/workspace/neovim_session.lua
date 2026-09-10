(function()
  local id, path, line, scratch, diff, review = unpack(_A)
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

  state.diffs = state.diffs or {}
  if path ~= nil and path ~= vim.NIL or scratch ~= nil and scratch ~= vim.NIL or review == true then
    local previous = state.diffs[tab]
    if previous then
      if vim.api.nvim_win_is_valid(previous.work) then
        vim.api.nvim_set_current_win(previous.work)
        vim.cmd('diffoff')
      end
      if vim.api.nvim_win_is_valid(previous.base)
          and vim.api.nvim_win_get_buf(previous.base) == previous.buffer then
        if #vim.api.nvim_tabpage_list_wins(tab) > 1 then
          vim.api.nvim_win_close(previous.base, true)
        else
          vim.cmd('diffoff')
        end
      end
      state.diffs[tab] = nil
    end

    -- Review activation focuses quickfix. Open targets in an editing window,
    -- never replace the quickfix buffer with a file or transcript.
    if vim.bo.buftype == 'quickfix' then
      local function can_edit(win)
        if not vim.api.nvim_win_is_valid(win)
            or vim.api.nvim_win_get_config(win).relative ~= '' then
          return false
        end
        local buf = vim.api.nvim_win_get_buf(win)
        local kind = vim.bo[buf].buftype
        return not vim.wo[win].previewwindow
            and (kind == '' or kind == 'nofile' and not vim.bo[buf].readonly)
      end
      local candidates = vim.api.nvim_tabpage_list_wins(tab)
      table.insert(candidates, 1, vim.fn.win_getid(vim.fn.winnr('#')))
      local target
      for _, win in ipairs(candidates) do
        if can_edit(win) then
          target = win
          break
        end
      end
      if target then
        vim.api.nvim_set_current_win(target)
      else
        vim.cmd('aboveleft new')
      end
    end
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
    local existing = vim.fn.bufnr(path)
    if existing ~= -1 then
      vim.cmd('hide buffer ' .. existing)
    else
      vim.cmd('hide edit ' .. vim.fn.fnameescape(path))
    end
    if line ~= nil and line ~= vim.NIL then
      vim.fn.cursor(line, 1)
    end
    if diff ~= nil and diff ~= vim.NIL then
      local work = vim.api.nvim_get_current_win()
      local filetype = vim.bo.filetype
      local lines = vim.fn.readfile(diff)
      vim.cmd('leftabove vnew')
      local base = vim.api.nvim_get_current_win()
      vim.bo.buftype = 'nofile'
      vim.bo.bufhidden = 'wipe'
      vim.bo.swapfile = false
      vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
      vim.api.nvim_buf_set_name(0, 'farcaster://HEAD/' .. id .. '/' .. vim.api.nvim_get_current_buf() .. '/' .. vim.fn.fnamemodify(path, ':t'))
      vim.bo.filetype = filetype
      vim.bo.modified = false
      vim.bo.modifiable = false
      vim.bo.readonly = true
      vim.cmd('diffthis')
      vim.api.nvim_set_current_win(work)
      vim.cmd('diffthis')
      state.diffs[tab] = { work = work, base = base, buffer = vim.api.nvim_win_get_buf(base) }
    end
  end
  return 0
end)()
