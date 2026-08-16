import {
	lstatSync,
	mkdirSync,
	readFileSync,
	renameSync,
	writeFileSync,
} from "node:fs";
import { resolve } from "node:path";

const PROJECT_POLICY_DIRECTORIES = [".pi", "extensions", "sandbox"] as const;

export function projectPolicyPath(cwd: string): string {
	return resolve(cwd, ...PROJECT_POLICY_DIRECTORIES, "sandbox.json");
}

export function readProjectPolicySource(cwd: string): string | null {
	assertSafeProjectPolicyDirectories(cwd, "supply sandbox policy");
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
	assertSafeProjectPolicyDirectories(cwd, "hold sandbox policy");
	if (expectedSourceText !== undefined && readProjectPolicySource(cwd) !== expectedSourceText) {
		throw new Error("Project sandbox policy changed while request_access was awaiting approval");
	}
	ensureProjectPolicyDirectories(cwd);
	assertSafeProjectPolicyDirectories(cwd, "hold sandbox policy");
	if (expectedSourceText !== undefined && readProjectPolicySource(cwd) !== expectedSourceText) {
		throw new Error("Project sandbox policy changed while request_access was awaiting approval");
	}
	const path = projectPolicyPath(cwd);
	const temporary = `${path}.${process.pid}.tmp`;
	writeFileSync(temporary, sourceText, { mode: 0o600 });
	renameSync(temporary, path);
}

function projectPolicyDirectories(cwd: string): string[] {
	const directories: string[] = [];
	for (let length = 1; length <= PROJECT_POLICY_DIRECTORIES.length; length += 1) {
		directories.push(resolve(cwd, ...PROJECT_POLICY_DIRECTORIES.slice(0, length)));
	}
	return directories;
}

function assertSafeProjectPolicyDirectories(cwd: string, action: string): void {
	for (const directory of projectPolicyDirectories(cwd)) {
		const metadata = lstatIfExists(directory);
		if (metadata?.isSymbolicLink()) {
			throw new Error(`A symlinked project control folder cannot ${action}: ${directory}`);
		}
		if (metadata && !metadata.isDirectory()) {
			throw new Error(`Project sandbox control path must be a directory: ${directory}`);
		}
	}
}

function ensureProjectPolicyDirectories(cwd: string): void {
	for (const directory of projectPolicyDirectories(cwd)) {
		if (!lstatIfExists(directory)) mkdirSync(directory, { mode: 0o700 });
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
