import { spawnSync } from "node:child_process";
import { basename, dirname, relative, resolve } from "node:path";
import { ansi, renderPatch, type DiffTheme } from "./pierre-renderer.ts";

const configuredGit = "@PI_DIFF_GIT@";
const git = process.env.PI_DIFF_GIT || (configuredGit.startsWith("@") ? "git" : configuredGit);

const colors = {
  accent: "#8ec07c",
  borderMuted: "#665c54",
  dim: "#928374",
  error: "#fb4934",
  toolDiffContext: "#a89984",
} as const;

const theme: DiffTheme = {
  fg(color, value) {
    return `${ansi({ color: colors[color] })}${value}\x1b[0m`;
  },
};

function fail(message: string): never {
  process.stderr.write(`pi-diff: ${message}\n`);
  process.exit(2);
}

function parseArguments(args: string[]): { left: string; right: string; width: number } {
  let width = Number.parseInt(process.env.COLUMNS ?? "", 10);
  const paths: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]!;
    if (arg === "--width") {
      const value = args[index + 1];
      if (!value) fail("--width needs a value");
      width = Number.parseInt(value, 10);
      index += 1;
    } else if (arg.startsWith("--width=")) {
      width = Number.parseInt(arg.slice("--width=".length), 10);
    } else if (arg === "--help" || arg === "-h") {
      process.stdout.write("Usage: pi-diff [--width COLUMNS] LEFT RIGHT\n");
      process.exit(0);
    } else if (arg.startsWith("-")) {
      fail(`unknown option: ${arg}`);
    } else {
      paths.push(arg);
    }
  }

  if (paths.length !== 2) fail("expected LEFT and RIGHT paths");
  return {
    left: resolve(paths[0]!),
    right: resolve(paths[1]!),
    width: Number.isFinite(width) ? Math.max(20, width) : 120,
  };
}

function commonParent(left: string, right: string): string {
  let parent = dirname(left);
  while (relative(parent, right).startsWith("..")) {
    const next = dirname(parent);
    if (next === parent) return process.cwd();
    parent = next;
  }
  return parent;
}

const { left, right, width } = parseArguments(process.argv.slice(2));
const cwd = commonParent(left, right);
const leftArg = relative(cwd, left) || basename(left);
const rightArg = relative(cwd, right) || basename(right);
const result = spawnSync(
  git,
  [
    "diff",
    "--no-index",
    "--no-ext-diff",
    "--no-color",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--",
    leftArg,
    rightArg,
  ],
  {
    cwd,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  },
);

if (result.error) fail(result.error.message);
if (result.status !== 0 && result.status !== 1) {
  const detail = result.stderr.trim();
  fail(detail || `git diff exited with status ${result.status}`);
}
if (result.status === 0 || !result.stdout) process.exit(0);

const lines = renderPatch(result.stdout, width, theme, [leftArg.replaceAll("\\", "/"), rightArg.replaceAll("\\", "/")]);
if (lines.length) process.stdout.write(`${lines.join("\n")}\n`);
