import assert from "node:assert/strict";
import test from "node:test";
import {
  sliceByColumn,
  stripAnsi,
  truncateToWidth,
  visibleWidth,
} from "../extensions/dense-tools/terminal-text.ts";

test("terminal width ignores ANSI color codes", () => {
  const colored = "\x1b[38;2;251;73;52mlet\x1b[0m";
  assert.equal(stripAnsi(colored), "let");
  assert.equal(visibleWidth(colored), 3);
});

test("terminal width counts wide and joined glyphs", () => {
  assert.equal(visibleWidth("a界b"), 4);
  assert.equal(visibleWidth("👨‍💻"), 2);
  assert.equal(visibleWidth("e\u0301"), 1);
});

test("column slices retain color without leaking it", () => {
  const colored = "\x1b[31mabcdef\x1b[0m";
  const slice = sliceByColumn(colored, 2, 3);
  assert.equal(stripAnsi(slice), "cde");
  assert.equal(visibleWidth(slice), 3);
  assert.match(slice, /^\x1b\[31m/);
  assert.match(slice, /\x1b\[0m$/);
});

test("truncation respects terminal columns", () => {
  assert.equal(stripAnsi(truncateToWidth("\x1b[32ma界bc\x1b[0m", 4)), "a界…");
  assert.equal(visibleWidth(truncateToWidth("abcdef", 4)), 4);
});
