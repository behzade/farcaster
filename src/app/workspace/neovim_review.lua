(function()
  local review, list_id, requested
  if type(_A) == 'table' then
    list_id, requested = unpack(_A)
    local previous = vim.fn.getqflist({id = list_id, context = 0, nr = 0})
    if previous.nr == 0 or type(previous.context) ~= 'table' or not previous.context.review then
      error('Review quickfix list is no longer available; reopen the review')
    end
    review = previous.context.review
    if requested < 1 or requested > #review.items then error('Invalid review location') end
    -- The app re-resolves the selected project-relative path at activation.
    if _A[3] then review.items[requested].path = _A[3] end
  else
    review = vim.json.decode(table.concat(vim.fn.readfile(_A), '\n'))
  end
  local entries, locations = {}, {}
  local first_valid
  for _, item in ipairs(review.items) do
    local first = type(item.start_line) == 'number' and item.start_line or 1
    local last = type(item.end_line) == 'number' and item.end_line or first
    local warning
    local buf = vim.fn.bufnr(item.path)
    local count
    if vim.fn.filereadable(item.path) ~= 1 then
      warning = 'Missing or unreadable file'
    elseif buf ~= -1 and vim.api.nvim_buf_is_loaded(buf) then
      count = vim.api.nvim_buf_line_count(buf)
      if vim.bo[buf].modified then warning = 'Buffer has unsaved edits; range may be stale' end
    else
      -- Count without loading an entire potentially large file into memory.
      local file = io.open(item.path, 'r')
      if file then
        count = 0
        for _ in file:lines() do
          count = count + 1
          if count >= last then break end
        end
        file:close()
        count = math.max(count, 1)
      else
        warning = 'Missing or unreadable file'
      end
    end
    if count and last > count then warning = 'Range exceeds current file; location is stale' end
    local valid = count ~= nil and last <= count
    if valid and not first_valid then first_valid = #entries + 1 end
    table.insert(locations, {valid = valid, warning = warning})
    table.insert(entries, {
      filename = item.path,
      lnum = first,
      end_lnum = last,
      text = (warning and ('[' .. warning .. '] ') or '') .. item.note,
      valid = valid and 1 or 0,
      type = warning and 'W' or '',
    })
  end
  -- Keep native quickfix navigation and history, but let Farcaster's sidebar
  -- display the list. Refresh the same list when selecting from that sidebar.
  local options = {
    title = 'Farcaster review: ' .. review.title,
    items = entries,
    context = { farcaster_review = true, advisory = true, review = review },
  }
  if list_id then options.id = list_id end
  vim.fn.setqflist({}, list_id and 'r' or ' ', options)
  local list = vim.fn.getqflist({id = list_id or 0, nr = 0})
  vim.cmd('chistory ' .. list.nr)
  vim.cmd('cclose')
  local selected = requested or first_valid
  if selected and locations[selected].valid then
    -- Pick the main editing pane, not a former quickfix pane now holding a
    -- normal buffer. Never replace a preview, floating, or read-only window.
    local target, area
    for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
      local buf = vim.api.nvim_win_get_buf(win)
      local kind = vim.bo[buf].buftype
      if vim.api.nvim_win_get_config(win).relative == ''
          and not vim.wo[win].previewwindow and not vim.bo[buf].readonly
          and (kind == '' or kind == 'nofile') then
        local size = vim.api.nvim_win_get_width(win) * vim.api.nvim_win_get_height(win)
        if not area or size > area then target, area = win, size end
      end
    end
    if target then vim.api.nvim_set_current_win(target)
    else vim.cmd('aboveleft new') end
    -- Explicitly open here: :cc may choose another window already showing the
    -- buffer. Keep the native list/index without delegating window selection.
    local item = review.items[selected]
    local buf = vim.fn.bufnr(item.path)
    if buf ~= -1 then vim.cmd('hide buffer ' .. buf)
    else vim.cmd('hide edit ' .. vim.fn.fnameescape(item.path)) end
    vim.fn.cursor(entries[selected].lnum, 1)
    vim.fn.setqflist({}, 'a', {id = list.id, idx = selected})
  else
    selected = nil
  end
  return vim.json.encode({
    list_id = list.id,
    selected = selected and selected - 1 or vim.NIL,
    locations = locations,
  })
end)()
