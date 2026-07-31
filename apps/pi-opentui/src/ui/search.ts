const resultLimit = 8

export const searchResultLimit = resultLimit

export const filterOptions = (
  options: ReadonlyArray<string>,
  query: string,
): ReadonlyArray<string> => {
  const needle = query.trim().toLocaleLowerCase()
  if (needle.length === 0) return options
  return options.filter((option) =>
    option.toLocaleLowerCase().includes(needle)
  )
}

export const nearbyOptions = (
  options: ReadonlyArray<string>,
  selectedIndex: number,
  limit = resultLimit,
): ReadonlyArray<{ readonly index: number; readonly option: string }> => {
  if (options.length === 0 || limit <= 0) return []

  const safeIndex = Math.max(0, Math.min(selectedIndex, options.length - 1))
  const size = Math.min(limit, options.length)
  const start = Math.max(
    0,
    Math.min(safeIndex - Math.floor(size / 2), options.length - size),
  )

  return options
    .slice(start, start + size)
    .map((option, offset) => ({ index: start + offset, option }))
}
