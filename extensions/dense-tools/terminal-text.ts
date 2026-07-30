const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;
const ANSI_TOKEN_PATTERN = /(\x1b\[[0-?]*[ -/]*[@-~])/g;
const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

function isFullWidth(codePoint: number): boolean {
  return codePoint >= 0x1100 && (
    codePoint <= 0x115f ||
    codePoint === 0x2329 ||
    codePoint === 0x232a ||
    (codePoint >= 0x2e80 && codePoint <= 0x303e) ||
    (codePoint >= 0x3040 && codePoint <= 0xa4cf) ||
    (codePoint >= 0xac00 && codePoint <= 0xd7a3) ||
    (codePoint >= 0xf900 && codePoint <= 0xfaff) ||
    (codePoint >= 0xfe10 && codePoint <= 0xfe19) ||
    (codePoint >= 0xfe30 && codePoint <= 0xfe6f) ||
    (codePoint >= 0xff00 && codePoint <= 0xff60) ||
    (codePoint >= 0xffe0 && codePoint <= 0xffe6) ||
    (codePoint >= 0x1b000 && codePoint <= 0x1b2ff) ||
    (codePoint >= 0x1f200 && codePoint <= 0x1f251) ||
    (codePoint >= 0x20000 && codePoint <= 0x3fffd)
  );
}

function graphemeWidth(value: string): number {
  if (!value || /^\p{Mark}+$/u.test(value)) return 0;
  if (/[\p{Extended_Pictographic}\u{1f1e6}-\u{1f1ff}]/u.test(value)) return 2;
  const codePoint = value.codePointAt(0);
  if (codePoint === undefined || codePoint < 0x20 || (codePoint >= 0x7f && codePoint < 0xa0)) return 0;
  return isFullWidth(codePoint) ? 2 : 1;
}

function graphemes(value: string): string[] {
  return [...segmenter.segment(value)].map(({ segment }) => segment);
}

export function stripAnsi(value: string): string {
  return value.replace(ANSI_PATTERN, "");
}

export function visibleWidth(value: string): number {
  return graphemes(stripAnsi(value)).reduce((width, grapheme) => width + graphemeWidth(grapheme), 0);
}

function nextAnsiState(state: string, sequence: string): string {
  if (sequence === "\x1b[0m" || sequence === "\x1b[m") return "";
  return sequence.endsWith("m") ? `${state}${sequence}` : state;
}

export function sliceByColumn(value: string, start: number, width: number): string {
  if (width <= 0) return "";

  const end = start + width;
  let column = 0;
  let ansiState = "";
  let output = "";
  let started = false;

  for (const token of value.split(ANSI_TOKEN_PATTERN)) {
    if (!token) continue;
    if (token.startsWith("\x1b[")) {
      ansiState = nextAnsiState(ansiState, token);
      if (started) output += token;
      continue;
    }

    for (const grapheme of graphemes(token)) {
      const graphemeColumns = graphemeWidth(grapheme);
      const nextColumn = column + graphemeColumns;
      if (!started && nextColumn > start) {
        started = true;
        output += ansiState;
      }
      if (started && column >= end) return output ? `${output}\x1b[0m` : "";
      if (started) output += grapheme;
      column = nextColumn;
    }
  }

  return output ? `${output}\x1b[0m` : "";
}

export function truncateToWidth(value: string, width: number, ellipsis = "…"): string {
  if (width <= 0) return "";
  if (visibleWidth(value) <= width) return value;
  const ellipsisWidth = visibleWidth(ellipsis);
  if (ellipsisWidth >= width) return sliceByColumn(ellipsis, 0, width);
  return `${sliceByColumn(value, 0, width - ellipsisWidth)}${ellipsis}`;
}
