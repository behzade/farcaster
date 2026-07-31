import { expect, test } from "bun:test"
import {
  clipboardSourceOrder,
  selectClipboardImageMimeType,
} from "../src/services/clipboard-source.ts"

test("chooses clipboard sources for Wayland, X11, other hosts, and Termux", () => {
  expect(
    clipboardSourceOrder(
      { WAYLAND_DISPLAY: "wayland-1" },
      "linux",
    ),
  ).toEqual(["wl-paste", "xclip", "native"])
  expect(clipboardSourceOrder({ DISPLAY: ":0" }, "linux")).toEqual([
    "native",
    "xclip",
  ])
  expect(clipboardSourceOrder({}, "darwin")).toEqual(["native"])
  expect(
    clipboardSourceOrder({ TERMUX_VERSION: "0.119" }, "linux"),
  ).toEqual([])
})

test("prefers supported image types and ignores MIME parameters", () => {
  expect(
    selectClipboardImageMimeType(
      "text/plain\nimage/jpeg\nimage/png; charset=binary",
    ),
  ).toBe("image/png")
  expect(selectClipboardImageMimeType("text/plain\ntext/html")).toBeUndefined()
})
