import assert from "node:assert/strict";
import test from "node:test";
import {
	backgroundJobSocket,
	isBackgroundJobSocket,
	isValidBackgroundJobName,
	modelVisibleBackgroundJobOutput,
	sandboxedJobCommand,
	shellJoin,
} from "./background-jobs.ts";
import { canonicalize } from "./io-permissions.ts";

test("the background job socket is reserved across macOS tmp aliases", () => {
	assert.equal(backgroundJobSocket({}), "/tmp/pi-agent-tmux.sock");
	assert.equal(
		isBackgroundJobSocket("/private/tmp/pi-agent-tmux.sock", canonicalize, {}),
		true,
	);
	assert.equal(isBackgroundJobSocket("/private/var/run/nix-daemon.socket", canonicalize, {}), false);
});

test("a custom absolute background socket is reserved", () => {
	const environment = { PI_BACKGROUND_TMUX_SOCKET: "/private/tmp/custom-pi.sock" };
	assert.equal(backgroundJobSocket(environment), "/private/tmp/custom-pi.sock");
	assert.equal(
		isBackgroundJobSocket("/private/tmp/custom-pi.sock", canonicalize, environment),
		true,
	);
	assert.equal(backgroundJobSocket({ PI_BACKGROUND_TMUX_SOCKET: "relative.sock" }), "/tmp/pi-agent-tmux.sock");
});

test("background job names stay in the broker namespace", () => {
	assert.equal(isValidBackgroundJobName("pi-app-dev"), true);
	assert.equal(isValidBackgroundJobName("app-dev"), false);
	assert.equal(isValidBackgroundJobName("pi-bad/name"), false);
	assert.equal(isValidBackgroundJobName(`pi-${"a".repeat(61)}`), false);
});

test("shell arguments stay distinct when a job command is wrapped", () => {
	assert.equal(shellJoin(["plain", "two words", "it's"]), "'plain' 'two words' 'it'\\''s'");
	const command = sandboxedJobCommand(
		"codex",
		["sandbox", "--", "bash", "-c", "printf '%s' \"$HOME\""],
		{ PATH: "/bin:/usr/bin", HOME: "/Users/test" },
	);
	assert.match(command, /^exec 'env' '-i'/);
	assert.match(command, /'HOME=\/Users\/test'/);
	assert.match(command, /'PATH=\/bin:\/usr\/bin'/);
	assert.match(command, /'printf '\\''%s'\\'' "\$HOME"'/);
});

test("background job reads are tail-bounded before first model emission", () => {
	const output = Array.from({ length: 2000 }, (_, index) => `line-${index}-${"x".repeat(40)}`).join("\n");
	assert.ok(Buffer.byteLength(output) > 50 * 1024);
	const visible = modelVisibleBackgroundJobOutput("read", output);

	assert.ok(Buffer.byteLength(visible) <= 50 * 1024);
	assert.ok(visible.split("\n").length <= 2000);
	assert.match(visible, /Background job read truncated/);
	assert.match(visible, /Full tmux history is preserved/);
	assert.doesNotMatch(visible, /line-0-/);
	assert.match(visible, /line-1999-/);
	assert.equal(modelVisibleBackgroundJobOutput("status", output), output);
});

test("background job read truncation keeps UTF-8 byte limits", () => {
	const visible = modelVisibleBackgroundJobOutput("read", `prefix-${"界".repeat(30_000)}`);
	assert.ok(Buffer.byteLength(visible) <= 50 * 1024);
	assert.doesNotMatch(visible, /�/);
	assert.match(visible, /界$/);
});
