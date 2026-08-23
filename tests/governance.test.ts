import assert from "node:assert/strict";
import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { NotificationCoalescer, osc9Sequence, preview } from "../extensions/lib/notification-core.ts";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

test("an approval request outranks a pending completion notice", () => {
  const notices = new NotificationCoalescer<{ type: string; priority: number }>();
  notices.push({ type: "agent-turn-complete", priority: 1 });
  notices.push({ type: "io-approval", priority: 2 });
  assert.deepEqual(notices.take(), { type: "io-approval", priority: 2 });
  assert.equal(notices.take(), undefined);
});

test("notification previews are short and safe for OSC 9", () => {
  assert.equal(preview("a\n  b"), "a b");
  assert.equal(osc9Sequence("hello\u001b]9;bad\u0007", false), "\u001b]9;hello ]9;bad \u0007");
  assert.equal(osc9Sequence("done", true), "\u001bPtmux;\u001b\u001b]9;done\u0007\u001b\\");
});

test("Pi GPUI vendors an offline Ghostty terminal against its host GPUI", async () => {
  const vendorRoot = resolve(
    repositoryRoot,
    "apps/pi-gpui/third_party/gpui-ghostty-e3025981",
  );
  const [provenance, workspace, zigManifest, buildScript] = await Promise.all([
    readFile(resolve(vendorRoot, "README.md"), "utf8"),
    readFile(resolve(vendorRoot, "Cargo.toml"), "utf8"),
    readFile(
      resolve(vendorRoot, "crates/ghostty_vt_sys/zig/build.zig.zon"),
      "utf8",
    ),
    readFile(resolve(vendorRoot, "crates/ghostty_vt_sys/build.rs"), "utf8"),
  ]);
  assert.match(provenance, /e3025981c6211dd7db2a825dc364ffb5d342f45e/);
  assert.match(provenance, /6d2dd585a5d87fa745d48188dd096ca6e63014d0/);
  assert.match(workspace, /path = "\.\.\/zed-gpui-cc053a4\/crates\/gpui"/);
  assert.match(zigManifest, /path = "\.\.\/\.\.\/\.\.\/vendor\/ziglyph"/);
  assert.doesNotMatch(zigManifest, /https?:\/\//);
  assert.match(buildScript, /--global-cache-dir/);
  await Promise.all([
    stat(resolve(vendorRoot, "LICENSE")),
    stat(resolve(vendorRoot, "vendor/ghostty/LICENSE")),
    stat(resolve(vendorRoot, "vendor/ziglyph/LICENSE")),
  ]);
  await Promise.all(
    [".git", ".zig-cache", "target", "node_modules"].map((name) =>
      assert.rejects(stat(resolve(vendorRoot, name))),
    ),
  );

  let bytes = 0;
  const pending = [vendorRoot];
  while (pending.length > 0) {
    const directory = pending.pop();
    assert.ok(directory);
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) {
        pending.push(path);
      } else if (entry.isFile()) {
        const file = await stat(path);
        assert.ok(
          file.size <= 1024 * 1024,
          `oversized vendored file: ${path}`,
        );
        bytes += file.size;
      }
    }
  }
  assert.ok(
    bytes < 10 * 1024 * 1024,
    `vendored terminal source is ${bytes} bytes`,
  );
});

test("Pi GPUI uses a narrow local Zed source snapshot", async () => {
  const appRoot = resolve(repositoryRoot, "apps/pi-gpui");
  const vendorRoot = resolve(appRoot, "third_party/zed-gpui-cc053a4");
  const expectedPackages = new Map([
    ["crates/collections/Cargo.toml", "collections"],
    ["crates/gpui/Cargo.toml", "gpui"],
    ["crates/gpui_linux/Cargo.toml", "gpui_linux"],
    ["crates/gpui_macos/Cargo.toml", "gpui_macos"],
    ["crates/gpui_macros/Cargo.toml", "gpui_macros"],
    ["crates/gpui_platform/Cargo.toml", "gpui_platform"],
    ["crates/gpui_shared_string/Cargo.toml", "gpui_shared_string"],
    ["crates/gpui_util/Cargo.toml", "gpui_util"],
    ["crates/gpui_web/Cargo.toml", "gpui_web"],
    ["crates/gpui_wgpu/Cargo.toml", "gpui_wgpu"],
    ["crates/gpui_windows/Cargo.toml", "gpui_windows"],
    ["crates/http_client/Cargo.toml", "http_client"],
    ["crates/media/Cargo.toml", "media"],
    ["crates/refineable/Cargo.toml", "refineable"],
    ["crates/refineable/derive_refineable/Cargo.toml", "derive_refineable"],
    ["crates/scheduler/Cargo.toml", "scheduler"],
    ["crates/sum_tree/Cargo.toml", "sum_tree"],
    ["crates/util_macros/Cargo.toml", "util_macros"],
    ["crates/zlog/Cargo.toml", "zlog"],
    ["crates/ztracing/Cargo.toml", "ztracing"],
    ["crates/ztracing_macro/Cargo.toml", "ztracing_macro"],
    ["tooling/perf/Cargo.toml", "perf"],
  ]);
  const lock = await readFile(resolve(appRoot, "Cargo.lock"), "utf8");
  assert.doesNotMatch(lock, /github\.com\/zed-industries\/zed(?:[?#\"]|$)/);

  const provenance = await readFile(resolve(vendorRoot, "README.md"), "utf8");
  assert.match(provenance, /cc053a4a6fa2fd0e8793201ed9099466af1be0b1/);
  await Promise.all([
    stat(resolve(vendorRoot, "LICENSE-APACHE")),
    stat(resolve(vendorRoot, "LICENSE-GPL")),
  ]);

  const forbidden = new Set([".git", "target", "tests", "examples", "benches"]);
  const manifests = new Map<string, string>();
  const manifestPaths: string[] = [];
  let bytes = 0;
  const pending = [{ path: vendorRoot, relativePath: "" }];
  while (pending.length > 0) {
    const directory = pending.pop();
    assert.ok(directory);
    for (const entry of await readdir(directory.path, { withFileTypes: true })) {
      assert.equal(forbidden.has(entry.name), false, `forbidden vendored directory: ${entry.name}`);
      const path = resolve(directory.path, entry.name);
      const relativePath = directory.relativePath
        ? `${directory.relativePath}/${entry.name}`
        : entry.name;
      assert.equal(entry.isSymbolicLink(), false, `vendored symlink: ${relativePath}`);
      if (entry.isDirectory()) {
        pending.push({ path, relativePath });
        continue;
      }
      assert.equal(entry.isFile(), true, `non-file in vendor tree: ${relativePath}`);
      bytes += (await stat(path)).size;
      if (entry.name !== "Cargo.toml") continue;

      manifestPaths.push(relativePath);
      const manifest = await readFile(path, "utf8");
      const packageSection = `${manifest}\n[__end__]\n`.match(
        /^\[package\][ \t]*\r?\n([\s\S]*?)(?=^\[)/m,
      )?.[1];
      if (packageSection === undefined) continue;
      const packageName = packageSection.match(/^name\s*=\s*"([^"]+)"\s*$/m)?.[1];
      assert.ok(packageName, `package name missing from ${relativePath}`);
      manifests.set(relativePath, packageName);
    }
  }
  assert.deepEqual(manifestPaths.sort(), ["Cargo.toml", ...expectedPackages.keys()].sort());
  assert.deepEqual([...manifests].sort(), [...expectedPackages].sort());
  assert.ok(bytes < 25 * 1024 * 1024, `vendored Zed source is ${bytes} bytes`);
});
