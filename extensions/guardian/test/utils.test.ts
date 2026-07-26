import { sep } from "node:path";
import { describe, expect, it } from "vitest";
import { deepEqual, stableStringify } from "../preflight/utils/json.js";
import { isPathWithin, isScopeOutsideWorkspace, toPosixPath } from "../preflight/utils/path.js";

describe("json utils", () => {
	it("deepEqual compares nested values", () => {
		expect(deepEqual({ a: [1, 2] }, { a: [1, 2] })).toBe(true);
		expect(deepEqual({ a: [1, 2] }, { a: [2, 1] })).toBe(false);
	});

	it("stableStringify sorts keys", () => {
		expect(stableStringify({ b: 1, a: 2 })).toBe("{\"a\":2,\"b\":1}");
	});
});

describe("path utils", () => {
	it("normalizes posix paths", () => {
		const value = ["a", "b"].join(sep);
		expect(toPosixPath(value)).toBe("a/b");
	});

	it("detects paths within base", () => {
		expect(isPathWithin("/workspace/src/file.ts", "/workspace")).toBe(true);
		expect(isPathWithin("/tmp/file.ts", "/workspace")).toBe(false);
	});

	it("detects scopes outside workspace", () => {
		const outside = isScopeOutsideWorkspace(["/tmp/file.txt"], "/workspace");
		const inside = isScopeOutsideWorkspace(["./src/index.ts"], "/workspace");
		expect(outside).toBe(true);
		expect(inside).toBe(false);
	});
});
