import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { hasTrustRequiringProjectResources } from "@earendil-works/pi-coding-agent";

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function project(): string {
  const root = mkdtempSync(join(tmpdir(), "pi-trust-resource-"));
  roots.push(root);
  mkdirSync(join(root, ".pi"));
  return root;
}

test("project sandbox policy requires project trust", () => {
  const root = project();
  writeFileSync(join(root, ".pi", "sandbox.json"), "{}\n");
  expect(hasTrustRequiringProjectResources(root)).toBe(true);
});

test("project-tools directory requires project trust", () => {
  const root = project();
  mkdirSync(join(root, ".pi", "project-tools"));
  expect(hasTrustRequiringProjectResources(root)).toBe(true);
});
