import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
	RepeatTracker,
	buildActionFingerprint,
	hasProjectGuardianRule,
	persistProjectGuardianRule,
} from "../preflight/guardian-rules.js";

const logDebug = () => {};

describe("guardian project rules", () => {
	it("remembers an exact action in the current project", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-rule-"));
		const action = buildActionFingerprint(
			{ id: "call-1", name: "bash", args: { command: "git reset --hard HEAD" } },
			cwd,
		);

		expect(hasProjectGuardianRule(cwd, action, logDebug, true)).toBe(false);
		expect(persistProjectGuardianRule(cwd, action, logDebug, true)).toBe(true);
		expect(hasProjectGuardianRule(cwd, action, logDebug, true)).toBe(true);

		const stored = JSON.parse(
			readFileSync(join(cwd, ".pi", "preflight", "settings.local.json"), "utf8"),
		) as { guardian: { allow: Array<{ fingerprint: string }> } };
		expect(stored.guardian.allow[0]?.fingerprint).toBe(action.fingerprint);
	});

	it("invalidates a script action when the script content changes", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-script-"));
		const script = join(cwd, "check.sh");
		writeFileSync(script, "#!/bin/sh\necho one\n");
		const before = buildActionFingerprint(
			{ id: "call-1", name: "bash", args: { command: "./check.sh" } },
			cwd,
		);
		writeFileSync(script, "#!/bin/sh\necho two\n");
		const after = buildActionFingerprint(
			{ id: "call-2", name: "bash", args: { command: "./check.sh" } },
			cwd,
		);

		expect(after.fingerprint).not.toBe(before.fingerprint);
	});

	it("stores rules at the repository root when Pi starts in a subdirectory", () => {
		const root = mkdtempSync(join(tmpdir(), "pi-guardian-root-"));
		const cwd = join(root, "crates", "one");
		mkdirSync(join(root, ".jj"));
		mkdirSync(cwd, { recursive: true });
		const action = buildActionFingerprint(
			{ id: "call-1", name: "bash", args: { command: "cargo test" } },
			cwd,
		);

		persistProjectGuardianRule(cwd, action, logDebug, true);
		expect(readFileSync(join(root, ".pi", "preflight", "settings.local.json"), "utf8")).toContain(
			action.fingerprint,
		);
	});

	it("does not replace an invalid project rule file", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-invalid-"));
		const file = join(cwd, ".pi", "preflight", "settings.local.json");
		mkdirSync(join(cwd, ".pi", "preflight"), { recursive: true });
		writeFileSync(file, "{invalid");
		const action = buildActionFingerprint(
			{ id: "call-1", name: "bash", args: { command: "cargo test" } },
			cwd,
		);

		expect(() => persistProjectGuardianRule(cwd, action, logDebug, true)).toThrow(
			"Refusing to replace invalid Guardian settings",
		);
		expect(readFileSync(file, "utf8")).toBe("{invalid");
	});

	it("ignores and refuses project rules when the project is not trusted", () => {
		const cwd = mkdtempSync(join(tmpdir(), "pi-guardian-untrusted-"));
		const action = buildActionFingerprint(
			{ id: "call-1", name: "bash", args: { command: "cargo test" } },
			cwd,
		);

		persistProjectGuardianRule(cwd, action, logDebug, true);
		expect(hasProjectGuardianRule(cwd, action, logDebug, false)).toBe(false);
		expect(() => persistProjectGuardianRule(cwd, action, logDebug, false)).toThrow(
			"untrusted project",
		);
	});

	it("counts repeated approved actions and can reset them", () => {
		const tracker = new RepeatTracker();
		expect(tracker.recordAllowed("same")).toBe(1);
		expect(tracker.recordAllowed("same")).toBe(2);
		tracker.reset("same");
		expect(tracker.recordAllowed("same")).toBe(1);
	});
});
