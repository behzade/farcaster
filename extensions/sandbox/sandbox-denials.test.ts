import assert from "node:assert/strict";
import test from "node:test";
import { parseFilesystemDenials } from "./sandbox-denials.ts";

test("parses macOS sandbox file denials", () => {
	const output = [
		"Sandbox: bash(123) deny(1) file-read-data /Users/behzad/Projects/other/input.txt",
		"Sandbox: cp(124) deny file-write-create \"/Users/behzad/Projects/other/output file.txt\"",
	].join("\n");
	assert.deepEqual(parseFilesystemDenials(output), [
		{ access: "read", path: "/Users/behzad/Projects/other/input.txt" },
		{ access: "write", path: "/Users/behzad/Projects/other/output file.txt" },
	]);
});

test("ignores ordinary command failures and deduplicates denials", () => {
	const denial = "Sandbox: cat(123) deny(1) file-read-data /private/data.txt";
	assert.deepEqual(parseFilesystemDenials(`cat: missing: No such file\n${denial}\n${denial}`), [
		{ access: "read", path: "/private/data.txt" },
	]);
});
