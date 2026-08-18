import assert from "node:assert/strict";
import test from "node:test";
import { Effect } from "effect";
import type {
	ChildSession,
	ChildSessionFactory,
	SendMode,
	StartRequest,
} from "./contract.ts";
import { SubagentCore } from "./core.ts";

interface PendingRun {
	resolve(output: string): void;
	reject(error: Error): void;
}

class FakeSession implements ChildSession {
	readonly id: string;
	readonly sessionFile: string;
	readonly provider = "authenticated";
	readonly model = "model-1";
	readonly effort = "high" as const;
	readonly sends: Array<{ message: string; mode: SendMode }> = [];
	pending!: PendingRun;
	streaming = true;
	disposals = 0;
	aborts = 0;
	#runPromise!: Promise<string>;

	constructor(id: string) {
		this.id = id;
		this.sessionFile = `/sessions/${id}.jsonl`;
		this.#resetPending();
	}

	#resetPending(): void {
		let resolvePromise!: (output: string) => void;
		let rejectPromise!: (error: Error) => void;
		this.#runPromise = new Promise<string>((ok, fail) => {
			resolvePromise = ok;
			rejectPromise = fail;
		});
		this.pending = {
			resolve: (output) => { resolvePromise(output); this.#resetPending(); },
			reject: (error) => { rejectPromise(error); this.#resetPending(); },
		};
	}

	isStreaming(): boolean { return this.streaming; }
	run(): Promise<string> { return this.#runPromise; }
	async send(message: string, mode: SendMode): Promise<void> { this.sends.push({ message, mode }); }
	async abort(): Promise<void> {
		this.aborts += 1;
		this.pending.reject(new Error("aborted"));
	}
	dispose(): void { this.disposals += 1; }
}

class FakeFactory implements ChildSessionFactory {
	readonly sessions: FakeSession[] = [];
	readonly requests: StartRequest[] = [];
	async create(request: StartRequest): Promise<ChildSession> {
		this.requests.push(request);
		const session = new FakeSession(`child-${this.sessions.length + 1}`);
		this.sessions.push(session);
		return session;
	}
}

const request = (prompt = "do the task"): StartRequest => ({
	prompt,
	cwd: "/workspace",
	parentSessionFile: "/sessions/parent.jsonl",
});

async function tick(): Promise<void> {
	await new Promise((resolve) => setImmediate(resolve));
}

test("start defaults to a fork and wait returns final assistant text exactly", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const started = await Effect.runPromise(core.start(request()));
	assert.equal(started.id, started.sessionId);
	assert.equal(started.context, "fork");
	assert.equal(factory.requests[0].context, "fork");

	const output = "Should I continue?\n\nThis is still the final answer.";
	factory.sessions[0].pending.resolve(output);
	const waited = await Effect.runPromise(core.wait([started.id]));
	assert.deepEqual(waited, {
		interruptedByUser: false,
		runs: [{ ...started, status: "idle", output }],
	});
	assert.equal(factory.sessions[0].disposals, 0);
	await Effect.runPromise(core.shutdown());
	assert.equal(factory.sessions[0].disposals, 1);
});

test("an idle child can be prompted again in the same persistent session", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const started = await Effect.runPromise(core.start(request()));
	factory.sessions[0].pending.resolve("first result");
	await Effect.runPromise(core.wait([started.id]));
	factory.sessions[0].streaming = false;
	const sent = await Effect.runPromise(core.send(started.id, "one more thing"));
	assert.equal(sent.mode, "prompt");
	assert.equal(sent.status, "running");
	factory.sessions[0].pending.resolve("second result");
	const waited = await Effect.runPromise(core.wait([started.id]));
	assert.equal(waited.interruptedByUser, false);
	if (!waited.interruptedByUser) assert.equal(waited.runs[0].output, "second result");
	assert.equal(factory.sessions.length, 1);
	assert.equal(factory.sessions[0].disposals, 0);
});

test("send infers steer while streaming and prompt while idle", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const started = await Effect.runPromise(core.start(request()));
	await Effect.runPromise(core.send(started.id, "redirect"));
	factory.sessions[0].streaming = false;
	await Effect.runPromise(core.send(started.id, "continue"));
	await assert.rejects(
		Effect.runPromise(core.send(started.id, "explicit", "steer")),
		/Cannot steer idle subagent/,
	);
	assert.deepEqual(factory.sessions[0].sends, [
		{ message: "redirect", mode: "steer" },
		{ message: "continue", mode: "prompt" },
	]);
});

test("parent input interrupts wait successfully and a later wait resumes", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const started = await Effect.runPromise(core.start(request()));
	const firstWait = Effect.runPromise(core.wait([started.id]));
	await tick();
	core.notifyUserInput();
	const interrupted = await firstWait;
	assert.equal(interrupted.interruptedByUser, true);
	if (interrupted.interruptedByUser) {
		assert.deepEqual(interrupted.pendingIds, [started.id]);
		assert.match(interrupted.instruction, /Address the user now.*call subagent_wait again/);
	}

	const secondWait = Effect.runPromise(core.wait([started.id]));
	factory.sessions[0].pending.resolve("done");
	const completed = await secondWait;
	assert.equal(completed.interruptedByUser, false);
	if (!completed.interruptedByUser) assert.equal(completed.runs[0].output, "done");
});

test("first and all waits preserve requested lifecycle semantics", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const one = await Effect.runPromise(core.start(request("one")));
	const two = await Effect.runPromise(core.start(request("two")));
	const firstWait = Effect.runPromise(core.wait([one.id, two.id], "first"));
	factory.sessions[1].pending.resolve("second first");
	const first = await firstWait;
	assert.equal(first.interruptedByUser, false);
	if (!first.interruptedByUser) assert.deepEqual(first.runs.map((run) => run.id), [two.id]);

	const allWait = Effect.runPromise(core.wait([one.id, two.id], "all"));
	factory.sessions[0].pending.resolve("first last");
	const all = await allWait;
	assert.equal(all.interruptedByUser, false);
	if (!all.interruptedByUser) assert.deepEqual(all.runs.map((run) => run.output), ["first last", "second first"]);
});

test("stop produces one terminal state for one persistent session", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const started = await Effect.runPromise(core.start(request()));
	const stopped = await Effect.runPromise(core.control("stop", started.id));
	assert.equal(stopped.status, "stopped");
	await tick();
	const status = await Effect.runPromise(core.control("status", started.id));
	assert.equal(status.status, "stopped");
	assert.equal(factory.sessions.length, 1);
	assert.equal(factory.sessions[0].aborts, 1);
	assert.equal(factory.sessions[0].disposals, 1);
});

test("blank children remain linked but do not require a fork source", async () => {
	const factory = new FakeFactory();
	const core = new SubagentCore(factory);
	const blank = await Effect.runPromise(core.start({
		prompt: "independent review",
		context: "blank",
		cwd: "/workspace",
	}));
	assert.equal(blank.context, "blank");
	await Effect.runPromise(core.control("stop", blank.id));
	await assert.rejects(Effect.runPromise(core.start({ prompt: "fork", cwd: "/workspace" })), /parent session is not persistent/);
});
