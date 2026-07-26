import assert from "node:assert/strict";
import test from "node:test";
import { parseFilesystemFailurePaths } from "./sandbox-failures.ts";

test("extracts exact paths from shell access failures", () => {
	const output = [
		"bash: line 1: /Users/behzad/Projects/other/output.txt: Operation not permitted",
		"cat: /Users/behzad/Projects/other/input file.txt: Permission denied",
	].join("\n");
	assert.deepEqual(parseFilesystemFailurePaths(output), [
		"/Users/behzad/Projects/other/output.txt",
		"/Users/behzad/Projects/other/input file.txt",
	]);
});

test("extracts quoted runtime paths after a failure keyword", () => {
	const output = "Error: EACCES: permission denied, open '/Users/behzad/Projects/other/file.txt'";
	assert.deepEqual(parseFilesystemFailurePaths(output), [
		"/Users/behzad/Projects/other/file.txt",
	]);
});

test("treats failures without an exact path as regular failures", () => {
	assert.deepEqual(parseFilesystemFailurePaths("curl: operation not permitted"), []);
	assert.deepEqual(parseFilesystemFailurePaths("command failed with code 1"), []);
});

test("ignores paths on lines without an access-failure keyword", () => {
	assert.deepEqual(
		parseFilesystemFailurePaths("missing: /Users/behzad/Projects/other/file.txt"),
		[],
	);
});
