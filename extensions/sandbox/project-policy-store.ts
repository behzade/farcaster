import {
	lstatSync,
	mkdirSync,
	readFileSync,
	renameSync,
	writeFileSync,
} from "node:fs";
import { resolve } from "node:path";

export function projectPolicyPath(cwd: string): string {
	return resolve(cwd, ".pi", "sandbox.json");
}

export function readProjectPolicySource(cwd: string): string | null {
	const controlRoot = resolve(cwd, ".pi");
	assertSafeProjectControlRoot(controlRoot, "supply sandbox policy");
	const path = projectPolicyPath(cwd);
	const metadata = lstatIfExists(path);
	if (!metadata) return null;
	if (metadata.isSymbolicLink()) {
		throw new Error(`A symlinked project sandbox policy is not allowed: ${path}`);
	}
	return readFileSync(path, "utf8");
}

/** Writes trusted host policy bytes only if the exact approved source is current. */
export function writeProjectPolicySource(
	cwd: string,
	sourceText: string,
	expectedSourceText?: string | null,
): void {
	const controlRoot = resolve(cwd, ".pi");
	assertSafeProjectControlRoot(controlRoot, "hold sandbox policy");
	if (expectedSourceText !== undefined && readProjectPolicySource(cwd) !== expectedSourceText) {
		throw new Error("Project sandbox policy changed while request_access was awaiting approval");
	}
	mkdirSync(controlRoot, { recursive: true, mode: 0o700 });
	assertSafeProjectControlRoot(controlRoot, "hold sandbox policy");
	if (expectedSourceText !== undefined && readProjectPolicySource(cwd) !== expectedSourceText) {
		throw new Error("Project sandbox policy changed while request_access was awaiting approval");
	}
	const path = projectPolicyPath(cwd);
	const temporary = `${path}.${process.pid}.tmp`;
	writeFileSync(temporary, sourceText, { mode: 0o600 });
	renameSync(temporary, path);
}

function assertSafeProjectControlRoot(controlRoot: string, action: string): void {
	if (lstatIfExists(controlRoot)?.isSymbolicLink()) {
		throw new Error(`A symlinked project control folder cannot ${action}: ${controlRoot}`);
	}
}

function lstatIfExists(path: string): ReturnType<typeof lstatSync> | undefined {
	try {
		return lstatSync(path);
	} catch (error) {
		if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
			return undefined;
		}
		throw error;
	}
}
