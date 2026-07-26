const MAX_PREVIEW_GRAPHEMES = 200;

export interface PrioritizedNotification {
  priority: number;
}

export class NotificationCoalescer<T extends PrioritizedNotification> {
  private pending: T | undefined;

  push(next: T): void {
    if (!this.pending || next.priority >= this.pending.priority) this.pending = next;
  }

  take(): T | undefined {
    const next = this.pending;
    this.pending = undefined;
    return next;
  }
}

export function preview(text: string): string {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (!normalized) return "Turn ended";
  const graphemes = Array.from(
    new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(normalized),
    ({ segment }) => segment,
  );
  return graphemes.length > MAX_PREVIEW_GRAPHEMES
    ? `${graphemes.slice(0, MAX_PREVIEW_GRAPHEMES).join("")}…`
    : normalized;
}

export function osc9Sequence(message: string, tmux: boolean): string {
  const safe = preview(message).replace(/[\u0000-\u001f\u007f]/g, " ");
  return tmux ? `\u001bPtmux;\u001b\u001b]9;${safe}\u0007\u001b\\` : `\u001b]9;${safe}\u0007`;
}
