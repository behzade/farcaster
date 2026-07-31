export const LARGE_PASTE_CHAR_LIMIT = 1_000
export const LARGE_PASTE_LINE_LIMIT = 10

export type PasteRequest =
  | { readonly kind: "clipboard" }
  | { readonly kind: "text"; readonly text: string }

export interface PasteInsertion {
  readonly kind: "inline" | "file"
  readonly text: string
}

export const isLargePaste = (text: string): boolean =>
  text.length > LARGE_PASTE_CHAR_LIMIT ||
  text.split("\n").length > LARGE_PASTE_LINE_LIMIT

