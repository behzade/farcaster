import assert from "node:assert/strict";
import test from "node:test";
import {
	isValidBackgroundJobName,
	modelVisibleBackgroundJobOutput,
} from "./background-jobs.ts";
import { backgroundKeyBytes } from "./native-background-jobs.ts";

test("background job names stay in the native broker namespace", () => {
	assert.equal(isValidBackgroundJobName("pi-server_1.test"), true);
	assert.equal(isValidBackgroundJobName("server"), false);
	assert.equal(isValidBackgroundJobName("pi-bad/name"), false);
});

test("background job reads are bounded before model emission", () => {
	const output = Array.from({ length: 4000 }, (_, index) => `line-${index}`).join("\n");
	const visible = modelVisibleBackgroundJobOutput("read", output);
	assert(Buffer.byteLength(visible) <= 50 * 1024);
	assert(visible.split("\n").length <= 2000);
	assert.match(visible, /truncated/);
	assert.match(visible, /line-3999$/);
});

test("background key names map to stdin bytes", () => {
	assert.deepEqual(backgroundKeyBytes(["hello", "Enter", "C-c"]), Buffer.from("hello\n\x03"));
});
