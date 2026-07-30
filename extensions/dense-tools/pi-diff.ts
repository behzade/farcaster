import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  unlinkSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, relative, resolve } from "node:path";

const configuredGit = "@PI_DIFF_GIT@";
const git = process.env.PI_DIFF_GIT || (configuredGit.startsWith("@") ? "git" : configuredGit);
const CACHE_VERSION = "pi-diff-v1";
const CACHE_MAX_ENTRIES = 128;
const CACHE_MAX_BYTES = 64 * 1024 * 1024;

const colors = {
  accent: "#8ec07c",
  borderMuted: "#665c54",
  dim: "#928374",
  error: "#fb4934",
  toolDiffContext: "#a89984",
} as const;

const theme = {
  fg(color: keyof typeof colors, value: string): string {
    const hex = colors[color as keyof typeof colors];
    const red = Number.parseInt(hex.slice(1, 3), 16);
    const green = Number.parseInt(hex.slice(3, 5), 16);
    const blue = Number.parseInt(hex.slice(5, 7), 16);
    return `\x1b[38;2;${red};${green};${blue}m${value}\x1b[0m`;
  },
};

function traceCache(state: "hit" | "miss"): void {
  if (process.env.PI_DIFF_CACHE_TRACE === "1") process.stderr.write(`pi-diff cache ${state}\n`);
}

function cacheDirectory(): string | undefined {
  if (process.env.PI_DIFF_CACHE === "0") return undefined;
  if (process.env.PI_DIFF_CACHE_DIR) return process.env.PI_DIFF_CACHE_DIR;
  const user = typeof process.getuid === "function" ? process.getuid() : "user";
  return `${tmpdir()}/pi-diff-${user}`;
}

function cachePath(patch: string, width: number): string | undefined {
  const directory = cacheDirectory();
  if (!directory) return undefined;
  const key = createHash("sha256")
    .update(CACHE_VERSION)
    .update("\0")
    .update(String(width))
    .update("\0")
    .update(patch)
    .digest("hex");
  return `${directory}/${key}.ansi`;
}

function readCache(path: string | undefined): string | undefined {
  if (!path) return undefined;
  try {
    const output = readFileSync(path, "utf8");
    const now = new Date();
    utimesSync(path, now, now);
    traceCache("hit");
    return output;
  } catch {
    traceCache("miss");
    return undefined;
  }
}

function trimCache(directory: string): void {
  const entries = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".ansi"))
    .map((entry) => {
      const path = `${directory}/${entry.name}`;
      const stat = statSync(path);
      return { path, modified: stat.mtimeMs, size: stat.size };
    })
    .sort((left, right) => left.modified - right.modified);
  let totalBytes = entries.reduce((total, entry) => total + entry.size, 0);
  while (entries.length > CACHE_MAX_ENTRIES || totalBytes > CACHE_MAX_BYTES) {
    const entry = entries.shift();
    if (!entry) break;
    unlinkSync(entry.path);
    totalBytes -= entry.size;
  }
}

function writeCache(path: string | undefined, output: string): void {
  if (!path || Buffer.byteLength(output) > CACHE_MAX_BYTES) return;
  try {
    const directory = dirname(path);
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    const temporary = `${path}.${process.pid}.tmp`;
    writeFileSync(temporary, output, { encoding: "utf8", mode: 0o600 });
    renameSync(temporary, path);
    trimCache(directory);
  } catch {
    // Rendering must still work when the cache is read-only or unavailable.
  }
}

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

const path = cachePath(result.stdout, width);
const cached = readCache(path);
if (cached !== undefined) {
  process.stdout.write(cached);
  process.exit(0);
}

const { renderPatch } = await import("./pierre-renderer.ts");
const lines = renderPatch(result.stdout, width, theme, [leftArg.replaceAll("\\", "/"), rightArg.replaceAll("\\", "/")]);
const output = lines.length ? `${lines.join("\n")}\n` : "";
writeCache(path, output);
process.stdout.write(output);
