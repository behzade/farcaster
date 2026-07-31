export interface PromptHistoryDraft {
  readonly text: string
  readonly cursorOffset: number
}

export class PromptHistory {
  private readonly entries: Array<string> = []
  private index = -1
  private draft: PromptHistoryDraft | undefined

  get isBrowsing(): boolean {
    return this.index >= 0
  }

  add(text: string): void {
    const trimmed = text.trim()
    if (trimmed.length === 0 || this.entries[0] === trimmed) return
    this.entries.unshift(trimmed)
    if (this.entries.length > 100) this.entries.pop()
  }

  navigate(
    direction: "older" | "newer",
    current: PromptHistoryDraft,
  ): PromptHistoryDraft | undefined {
    const nextIndex =
      this.index + (direction === "older" ? 1 : -1)
    if (nextIndex < -1 || nextIndex >= this.entries.length) return undefined

    if (this.index === -1 && nextIndex >= 0) this.draft = current
    this.index = nextIndex
    if (this.index === -1) {
      const draft = this.draft ?? { text: "", cursorOffset: 0 }
      this.draft = undefined
      return draft
    }

    const text = this.entries[this.index] ?? ""
    return {
      text,
      cursorOffset: text.length,
    }
  }

  exit(): void {
    this.index = -1
    this.draft = undefined
  }
}
