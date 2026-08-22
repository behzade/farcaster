import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
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

test("project-tools directory requires project trust", () => {
  const root = project();
  mkdirSync(join(root, ".pi", "project-tools"));
  expect(hasTrustRequiringProjectResources(root)).toBe(true);
});
