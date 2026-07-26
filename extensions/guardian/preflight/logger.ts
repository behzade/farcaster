import { appendFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import type { DebugLogger, PreflightConfig } from "./types.js";

const DEBUG_LOG_DIR = join(".pi", "preflight", "logs");
const DEBUG_LOG_FILE = "preflight-debug.log";

export function createDebugLogger(ctx: ExtensionContext, config: PreflightConfig): DebugLogger {
	if (!config.debug) {
		return () => {};
	}

	const logPath = getDebugLogPath(ctx.cwd);
	const writeLine = (message: string): void => {
		try {
			mkdirSync(dirname(logPath), { recursive: true });
			appendFileSync(logPath, formatLogEntry(message));
		} catch (error) {
			if (!ctx.hasUI) {
				const errorMessage = error instanceof Error ? error.message : String(error);
				console.log(`[bo-pi] Failed to write debug log: ${errorMessage}`);
			}
		}
		if (!ctx.hasUI) {
			console.log(`[bo-pi] ${message}`);
		}
	};

	writeLine(`Debug logging enabled. File: ${logPath}`);
	return writeLine;
}

function formatLogEntry(message: string): string {
	const timestamp = new Date().toISOString();
	const lines = message.split(/\r?\n/);
	return `${lines.map((line) => `[${timestamp}] ${line}`).join("\n")}\n`;
}

function getDebugLogPath(cwd: string): string {
	return join(cwd, DEBUG_LOG_DIR, DEBUG_LOG_FILE);
}
