import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";

const installedPackage = process.env.PI_SUBAGENTS_PACKAGE;

test("packaged subagents do not treat scoped edit exclusions as review-only tasks", {
  skip: installedPackage ? false : "PI_SUBAGENTS_PACKAGE is set by the Nix integration check",
}, async () => {
  const module = await import(
    pathToFileURL(join(installedPackage!, "src", "runs", "shared", "pi-args.ts")).href
  ) as {
    buildPiArgs(input: {
      baseArgs: string[];
      task: string;
      sessionEnabled: boolean;
      inheritProjectContext: boolean;
      inheritSkills: boolean;
    }): { env: Record<string, string> };
  };
  const build = (task: string) => module.buildPiArgs({
    baseArgs: [],
    task,
    sessionEnabled: false,
    inheritProjectContext: true,
    inheritSkills: true,
  }).env.PI_SUBAGENT_READ_ONLY_TASK;

  assert.equal(build("Implement and test the fix. Do not edit third_party or GPUI files."), "0");
  assert.equal(build('Fix the classifier when a task mentions "do not edit files".'), "0");
  assert.equal(build("Perform a read-only review of these changes."), "1");
  assert.equal(build("Read-only review. Do not edit any files."), "1");
});

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
