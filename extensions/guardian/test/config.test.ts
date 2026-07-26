import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadPersistentConfig, parseConfig, parseContextValue } from "../preflight/config.js";

afterEach(() => {
	vi.unstubAllEnvs();
});

describe("config parsing", () => {
	it("normalizes context messages", () => {
		const zero = parseConfig({ contextMessages: 0 });
		expect(zero.contextMessages).toBe(1);

		const negative = parseConfig({ contextMessages: -5 });
		expect(negative.contextMessages).toBe(-1);
	});

	it("supports legacy flags", () => {
		const destructiveOnly = parseConfig({ approveDestructiveOnly: true });
		expect(destructiveOnly.approvalMode).toBe("destructive");

		const disabled = parseConfig({ enabled: false });
		expect(disabled.approvalMode).toBe("off");
	});

	it("parses context value", () => {
		expect(parseContextValue("full")).toBe(-1);
		expect(parseContextValue("3")).toBe(3);
		expect(parseContextValue("0")).toBeUndefined();
	});

	it("gives the managed config priority over mutable defaults", () => {
		const agentDir = mkdtempSync(join(tmpdir(), "pi-guardian-config-"));
		mkdirSync(join(agentDir, "extensions", "bo-pi"), { recursive: true });
		writeFileSync(
			join(agentDir, "extensions", "bo-pi", "preflight.json"),
			JSON.stringify({ approvalMode: "off", repeatThreshold: 9 }),
		);
		writeFileSync(
			join(agentDir, "extensions", "guardian.json"),
			JSON.stringify({
				approvalMode: "auto",
				model: { provider: "openai-codex", id: "gpt-5.6-terra" },
				reasoning: "low",
				repeatThreshold: 2,
			}),
		);
		vi.stubEnv("PI_CODING_AGENT_DIR", agentDir);

		const config = loadPersistentConfig();
		expect(config.approvalMode).toBe("auto");
		expect(config.model).toEqual({ provider: "openai-codex", id: "gpt-5.6-terra" });
		expect(config.reasoning).toBe("low");
		expect(config.repeatThreshold).toBe(2);
	});
});
