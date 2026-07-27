import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const themePath = new URL("../themes/gruvbox-dark-hard.json", import.meta.url);
const denseToolsPath = new URL("../extensions/dense-tools/index.ts", import.meta.url);
const pierreEditPath = new URL("../extensions/dense-tools/pierre-edit.ts", import.meta.url);
const chatLayoutPath = new URL("../extensions/dense-tools/chat-layout.ts", import.meta.url);
const sandboxPath = new URL("../extensions/sandbox/index.ts", import.meta.url);

const requiredColors = [
  "accent", "border", "borderAccent", "borderMuted", "success", "error", "warning",
  "muted", "dim", "text", "thinkingText", "selectedBg", "userMessageBg",
  "userMessageText", "customMessageBg", "customMessageText", "customMessageLabel",
  "toolPendingBg", "toolSuccessBg", "toolErrorBg", "toolTitle", "toolOutput",
  "mdHeading", "mdLink", "mdLinkUrl", "mdCode", "mdCodeBlock", "mdCodeBlockBorder",
  "mdQuote", "mdQuoteBorder", "mdHr", "mdListBullet", "toolDiffAdded",
  "toolDiffRemoved", "toolDiffContext", "syntaxComment", "syntaxKeyword",
  "syntaxFunction", "syntaxVariable", "syntaxString", "syntaxNumber", "syntaxType",
  "syntaxOperator", "syntaxPunctuation", "thinkingOff", "thinkingMinimal", "thinkingLow",
  "thinkingMedium", "thinkingHigh", "thinkingXhigh", "thinkingMax", "bashMode",
] as const;

const canonicalPalette = {
  bg0Hard: "#1d2021",
  bg0: "#282828",
  bg1: "#3c3836",
  bg2: "#504945",
  bg3: "#665c54",
  bg4: "#7c6f64",
  gray: "#928374",
  fg4: "#a89984",
  fg3: "#bdae93",
  fg2: "#d5c4a1",
  fg1: "#ebdbb2",
  fg0: "#fbf1c7",
  red: "#cc241d",
  green: "#98971a",
  yellow: "#d79921",
  blue: "#458588",
  purple: "#b16286",
  aqua: "#689d6a",
  orange: "#d65d0e",
  brightRed: "#fb4934",
  brightGreen: "#b8bb26",
  brightYellow: "#fabd2f",
  brightBlue: "#83a598",
  brightPurple: "#d3869b",
  brightAqua: "#8ec07c",
  brightOrange: "#fe8019",
};

test("Gruvbox dark hard uses the canonical palette and every Pi color", async () => {
  const theme = JSON.parse(await readFile(themePath, "utf8"));
  assert.equal(theme.name, "gruvbox-dark-hard");
  assert.deepEqual(theme.vars, canonicalPalette);
  for (const color of requiredColors) assert.ok(color in theme.colors, `missing ${color}`);
  assert.equal(theme.colors.userMessageBg, "bg0");
  assert.equal(theme.colors.userMessageText, "fg1");
  assert.equal(theme.colors.border, "bg4");
  assert.equal(theme.colors.text, "fg2");
  assert.equal(theme.colors.mdHeading, "fg0");
  assert.equal(theme.colors.mdLink, "brightAqua");
  assert.equal(theme.colors.mdListBullet, "brightAqua");
  assert.equal(theme.colors.syntaxKeyword, "fg0");
  assert.equal(theme.colors.syntaxString, "fg4");
  assert.equal(theme.colors.thinkingMax, "fg1");
  assert.equal(theme.export.pageBg, canonicalPalette.bg0Hard);
});

test("dense tools leave bash to the sandbox", async () => {
  const denseTools = await readFile(denseToolsPath, "utf8");
  const sandbox = await readFile(sandboxPath, "utf8");
  assert.doesNotMatch(denseTools, /createBashTool|name:\s*["']bash["']/);
  assert.match(sandbox, /renderShell:\s*["']self["']/);
});

test("sandbox prompts rejected path tool calls without exposing a model tool", async () => {
  const sandbox = await readFile(sandboxPath, "utf8");
  assert.doesNotMatch(sandbox, /name:\s*["']request_io_permission["']/);
  assert.match(sandbox, /promptForToolPermission/);
  assert.match(sandbox, /await promptForToolPermission\(permission, event, ctx\)/);
});

test("one-time network rights stay on one command", async () => {
  const sandbox = await readFile(sandboxPath, "utf8");
  assert.match(sandbox, /kind:\s*Type\.Literal\(["']network_host["']\)/);
  assert.match(sandbox, /declaredNetworkPermissions/);
  assert.doesNotMatch(sandbox, /oneShotNetworkPermissions/);
});

test("dense reads keep group state and hide follower rows", async () => {
  const source = await readFile(denseToolsPath, "utf8");
  assert.match(source, /readGroupsById/);
  assert.match(source, /group\.entries\[0\] !== entry/);
  assert.match(source, /return new Container\(\)/);
  assert.match(source, /tool_execution_start/);
  assert.match(source, /message_end/);
});

test("Pierre edit renderer wraps responsive split diffs and keeps change backgrounds", async () => {
  const source = await readFile(pierreEditPath, "utf8");
  assert.match(source, /width >= 120/);
  assert.match(source, /renderSplit/);
  assert.match(source, /COLLAPSED_DIFF_ROWS = 30/);
  assert.match(source, /capRows\(indexes, expanded\)/);
  assert.match(source, /cachedWidth/);
  assert.match(source, /cachedLines/);
  assert.match(source, /if \(patchChanged\) this\.rebuild\(\)/);
  assert.match(source, /if \(this\.cachedLines && this\.cachedWidth === width\) return this\.cachedLines/);
  assert.match(source, /sliceByColumn/);
  assert.match(source, /function wrapByColumns/);
  assert.match(source, /function wrapCell/);
  assert.match(source, /capped\.visible\.flatMap/);
  assert.match(source, /Math\.max\(oldCell\.lines\.length, newCell\.lines\.length\)/);
  assert.match(source, /oldCell\.lines\[lineIndex\] \?\? fitCell/);
  assert.match(source, /newCell\.lines\[lineIndex\] \?\? fitCell/);
  assert.match(source, /inlineBackground/);
  assert.match(source, /frameDiff/);
  assert.match(source, /DIFF_BACKGROUND = "#191b1c"/);
  assert.match(source, /theme\.fg\("borderMuted", "│"\)/);
  assert.doesNotMatch(source, /" │ "/);
  assert.doesNotMatch(source, /theme\.fg\("muted", "before"\)/);
  assert.match(source, /#412724/);
  assert.match(source, /#363922/);
});

test("chat layout leaves messages unframed and gives chat rows a margin", async () => {
  const source = await readFile(chatLayoutPath, "utf8");
  assert.match(source, /UserMessageComponent/);
  assert.match(source, /AssistantMessageComponent/);
  assert.match(source, /ToolExecutionComponent/);
  assert.match(source, /EXTRA_LEFT_MARGIN = 1/);
  assert.match(source, /COMPOSER_PADDING = 2/);
  assert.match(source, /setComposerMargin/);
  assert.match(source, /setPaddingX/);
  assert.match(source, /return lines\.map\(indent\)/);
  assert.doesNotMatch(source, /USER_RULE|ASSISTANT_RULE/);
  assert.doesNotMatch(source, /function rule|"around"|"plain"/);
  assert.doesNotMatch(source, /"before"/);
});
