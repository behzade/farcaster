import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

const installedPackage = process.env.PI_SUBAGENTS_PACKAGE;

test("packaged subagents resolve the Nix permission-system extension beside its runtime state directory", {
  skip: installedPackage ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const root = await mkdtemp(join(tmpdir(), "pi-subagents-permission-system-"));
  const agentDir = join(root, "agent");
  const stateDir = join(agentDir, "extensions", "pi-permission-system");
  const extensionDir = join(agentDir, "extensions", "permission-system");
  const entryPath = join(extensionDir, "src", "index.ts");
  const originalAgentDir = process.env.PI_CODING_AGENT_DIR;

  try {
    await mkdir(stateDir, { recursive: true });
    await writeFile(join(stateDir, "config.json"), "{}\n");
    await mkdir(join(extensionDir, "src"), { recursive: true });
    await writeFile(
      join(extensionDir, "package.json"),
      JSON.stringify({
        name: "@gotgenes/pi-permission-system",
        pi: { extensions: ["./src/index.ts"] },
      }),
    );
    await writeFile(entryPath, "export default () => {};\n");
    process.env.PI_CODING_AGENT_DIR = agentDir;

    const module = await import(
      pathToFileURL(join(installedPackage!, "src", "runs", "shared", "pi-args.ts")).href
    ) as { resolvePermissionSystemExtension(): string | undefined };

    assert.equal(module.resolvePermissionSystemExtension(), entryPath);
  } finally {
    if (originalAgentDir === undefined) delete process.env.PI_CODING_AGENT_DIR;
    else process.env.PI_CODING_AGENT_DIR = originalAgentDir;
    await rm(root, { recursive: true, force: true });
  }
});
