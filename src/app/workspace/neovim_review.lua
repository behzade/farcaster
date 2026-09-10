(function()
  local review = vim.json.decode(table.concat(vim.fn.readfile(_A), '\n'))
  local entries = {}
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
    table.insert(entries, {
      filename = item.path,
      lnum = first,
      end_lnum = last,
      text = (warning and ('[' .. warning .. '] ') or '') .. item.note,
      valid = count and last <= count and 1 or 0,
      type = warning and 'W' or '',
    })
  end
  -- A new list preserves the previous list in :colder history. Do not jump
  -- automatically: missing entries must never create files as a side effect.
  vim.fn.setqflist({}, ' ', {
    title = 'Farcaster review: ' .. review.title,
    items = entries,
    context = { farcaster_review = true, advisory = true },
  })
  vim.cmd('botright copen')
  return 0
end)()
