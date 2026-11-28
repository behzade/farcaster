local function open_review(review, requested, path)
  local list_id
  if type(review) == 'number' then
    list_id = review
    local previous = vim.fn.getqflist({id = list_id, context = 0, nr = 0})
    if previous.nr == 0 or type(previous.context) ~= 'table' or not previous.context.review then
      error('Review quickfix list is no longer available; reopen the review')
    end
    review = previous.context.review
    if requested < 1 or requested > #review.items then error('Invalid review location') end
    if path then review.items[requested].path = path end
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
    local item = review.items[selected]
    local buf = vim.fn.bufnr(item.path)
    if buf ~= -1 then vim.cmd('hide buffer ' .. buf)
    else vim.cmd('hide edit ' .. vim.fn.fnameescape(item.path)) end
    vim.fn.cursor(entries[selected].lnum, 1)
    vim.fn.setqflist({}, 'a', {id = list.id, idx = selected})
  else
    selected = nil
  end
  local group = vim.api.nvim_create_augroup('FarcasterReviewSelection', {clear = true})
  local published = selected
  local published_channel = vim.g.farcaster_rpc_channel
  local function publish_selection()
    local channel = vim.g.farcaster_rpc_channel
    local current = vim.fn.getqflist({id = 0, idx = 0, context = 0})
    if not channel or current.id ~= list.id or type(current.context) ~= 'table'
        or not current.context.farcaster_review or current.idx < 1
        or (current.idx == published and channel == published_channel) then
      return
    end
    local ok, sent = pcall(vim.rpcnotify, channel, 'farcaster_review_selection', {
      list_id = current.id,
      selected = current.idx - 1,
    })
    if ok and sent then
      published = current.idx
      published_channel = channel
    end
  end
  vim.api.nvim_create_autocmd({'BufEnter', 'CursorMoved', 'CmdlineLeave'}, {
    group = group,
    callback = vim.schedule_wrap(publish_selection),
  })
  return {
    list_id = list.id,
    selected = selected and selected - 1 or vim.NIL,
    locations = locations,
  }
end

return open_review(...)
