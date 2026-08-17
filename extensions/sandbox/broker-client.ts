import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { accessSync, constants } from "node:fs";
import { Effect, Exit, Schema, Scope } from "effect";

const PROTOCOL_VERSION = 4;
export const MAX_BROKER_FRAME_BYTES = 1024 * 1024;
const READY_TIMEOUT_MS = 5_000;
const SHUTDOWN_TIMEOUT_MS = 5_000;
const BROKER_ERROR_CODES = new Set([
	"backend_unavailable",
	"duplicate_command_id",
	"invalid_request",
	"policy_rejected",
	"command_start_failed",
	"cancelled",
	"protocol_error",
	"not_found",
]);

export class SandboxBrokerError extends Schema.TaggedError<SandboxBrokerError>()(
	"SandboxBrokerError",
	{
		message: Schema.String,
		cause: Schema.optional(Schema.Defect()),
	},
) {}

const brokerError = (cause: unknown) => new SandboxBrokerError({
	message: cause instanceof Error ? cause.message : String(cause),
	cause,
});

export interface BrokerFilesystemRight {
	access: "read" | "write";
	path: string;
	scope: "file" | "tree";
	missing_path: "reject" | "create_file" | "create_tree";
}

export interface BrokerFilesystemDeny {
	access: "read" | "write" | "read_write";
	pattern: string;
	scope: "file" | "tree" | "glob";
}

export interface BrokerDenial {
	operation: string;
	path: string | null;
	process: string | null;
}

export interface BrokerExecResult {
	exitCode: number | null;
	denials: BrokerDenial[];
	denialsComplete: boolean;
}

export interface BrokerExecRequest {
	type: "exec";
	id: string;
	command: { program: string; args: string[] };
	cwd: string;
	env: Record<string, string>;
	timeout_ms: number | null;
	interactive?: boolean;
	policy: {
		base_rights: BrokerFilesystemRight[];
		grants: BrokerFilesystemRight[];
		denies: BrokerFilesystemDeny[];
		network:
			| { mode: "blocked" }
			| { mode: "loopback" }
			| {
					mode: "proxy";
					tcp_port: number;
					unix_socket: string;
					allow_local_binding: boolean;
			  };
		unix_socket_roots: string[];
		output_limit_bytes: number;
	};
}

type BrokerRequest =
	| BrokerExecRequest
	| { type: "cancel"; id: string }
	| { type: "write_stdin"; id: string; data_base64: string }
	| { type: "shutdown" };

type BrokerEvent =
	| {
			type: "ready";
			version: number;
			platform: string;
			backend: string;
			can_exec: boolean;
			max_frame_bytes: number;
	  }
	| { type: "started"; id: string; pid: number }
	| { type: "stdout" | "stderr"; id: string; sequence: number; data_base64: string }
	| { type: "denials"; id: string; items: BrokerDenial[]; complete: boolean }
	| {
			type: "exit";
			id: string;
			code: number | null;
			signal: number | null;
			timed_out: boolean;
			cancelled: boolean;
			output_truncated: boolean;
	  }
	| { type: "error"; id: string | null; code: string; message: string };

interface PendingExec {
	onData: (data: Buffer) => void;
	resume: (effect: Effect.Effect<BrokerExecResult, SandboxBrokerError>) => void;
	signal: AbortSignal;
	onAbort: () => void;
	cancelSent: boolean;
	started: boolean;
	stdoutSequence: number;
	stderrSequence: number;
	timeoutSeconds?: number;
	denials?: BrokerDenial[];
	denialsComplete: boolean;
	onStarted?: (pid: number) => void;
}

export class FramedJsonDecoder {
	#pending = Buffer.alloc(0);

	push(chunk: Buffer): unknown[] {
		this.#pending = Buffer.concat([this.#pending, chunk]);
		const messages: unknown[] = [];
		while (this.#pending.length >= 4) {
			const size = this.#pending.readUInt32BE(0);
			if (size === 0) throw new Error("Broker sent an empty frame");
			if (size > MAX_BROKER_FRAME_BYTES) throw new Error(`Broker frame exceeds ${MAX_BROKER_FRAME_BYTES} bytes`);
			if (this.#pending.length < size + 4) break;
			const body = this.#pending.subarray(4, size + 4);
			this.#pending = this.#pending.subarray(size + 4);
			messages.push(JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body)));
		}
		return messages;
	}

	finish(): void {
		if (this.#pending.length > 0) throw new Error("Broker closed with a partial frame");
	}
}

export function encodeBrokerFrame(message: unknown): Buffer {
	const body = Buffer.from(JSON.stringify(message), "utf8");
	if (body.length === 0 || body.length > MAX_BROKER_FRAME_BYTES) throw new Error("Broker request frame has an invalid size");
	const frame = Buffer.allocUnsafe(body.length + 4);
	frame.writeUInt32BE(body.length, 0);
	body.copy(frame, 4);
	return frame;
}

export class SandboxBrokerClient {
	readonly #child: ChildProcessWithoutNullStreams;
	readonly #decoder = new FramedJsonDecoder();
	readonly #pending = new Map<string, PendingExec>();
	readonly #expectedPlatform: NodeJS.Platform;
	readonly #onStdout: (chunk: Buffer) => void;
	readonly #onStderr: () => void;
	readonly #onError: (error: Error) => void;
	readonly #onClose: (code: number | null, signal: NodeJS.Signals | null) => void;
	#readyResume?: (effect: Effect.Effect<void, SandboxBrokerError>) => void;
	#ready = false;
	#requiresDenials = false;
	#failed = false;
	#exited = false;
	#shutdownStarted = false;
	#ownerScope?: Scope.Closeable;

	private constructor(child: ChildProcessWithoutNullStreams, expectedPlatform: NodeJS.Platform) {
		this.#child = child;
		this.#expectedPlatform = expectedPlatform;
		this.#onStdout = (chunk) => this.#onChunk(chunk);
		this.#onStderr = () => { /* Host diagnostics stay off the model-visible stream. */ };
		this.#onError = (error) => this.#fail(error);
		this.#onClose = (code, signal) => {
			this.#exited = true;
			try {
				this.#decoder.finish();
			} catch (error) {
				this.#fail(asError(error));
			}
			this.#fail(new Error(`Sandbox broker exited (${code ?? "no code"}${signal ? `, ${signal}` : ""})`));
		};
		child.stdout.on("data", this.#onStdout);
		child.stderr.on("data", this.#onStderr);
		child.once("error", this.#onError);
		child.once("close", this.#onClose);
	}

	static readonly acquire = Effect.fn("SandboxBrokerClient.acquire")(function* (
		path: string,
		expectedPlatform: NodeJS.Platform = process.platform,
		developmentCacheRoot?: string,
	) {
		const client = yield* Effect.acquireRelease(
			Effect.try({
				try: () => {
					accessSync(path, constants.X_OK);
					return new SandboxBrokerClient(spawn(path, [], {
						stdio: ["pipe", "pipe", "pipe"],
						env: buildBrokerEnvironment(developmentCacheRoot),
					}), expectedPlatform);
				},
				catch: brokerError,
			}),
			(client) => client.shutdownEffect().pipe(
				Effect.ensuring(Effect.sync(() => client.#detachListeners())),
				Effect.catch(() => Effect.void),
			),
		);
		yield* client.#waitUntilReadyEffect;
		return client;
	});

	/** Promise boundary adapter. The returned client owns the scope closed by shutdown(). */
	static start(
		path: string,
		expectedPlatform: NodeJS.Platform = process.platform,
		developmentCacheRoot?: string,
	): Promise<SandboxBrokerClient> {
		return Effect.runPromise(Effect.gen(function* () {
			const scope = yield* Scope.make();
			const acquire = SandboxBrokerClient.acquire(path, expectedPlatform, developmentCacheRoot).pipe(Scope.provide(scope));
			const client = yield* acquire.pipe(Effect.onExit((exit) => Exit.isFailure(exit) ? Scope.close(scope, exit) : Effect.void));
			client.#ownerScope = scope;
			return client;
		}));
	}

	readonly execEffect = Effect.fn("SandboxBrokerClient.exec")( (
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
		onStarted?: (pid: number) => void,
	) => {
		if (!this.#ready || this.#failed || this.#exited) return Effect.fail(brokerError("Sandbox broker is not ready"));
		if (this.#pending.has(request.id)) return Effect.fail(brokerError(`Duplicate broker command ID: ${request.id}`));
		return Effect.callback<BrokerExecResult, SandboxBrokerError>((resume, signal) => {
			const pending: PendingExec = {
				onData,
				resume,
				signal,
				onAbort: () => this.#cancelPending(request.id),
				cancelSent: false,
				started: false,
				stdoutSequence: 0,
				stderrSequence: 0,
				denialsComplete: false,
				onStarted,
				timeoutSeconds: request.timeout_ms === null ? undefined : request.timeout_ms / 1000,
			};
			signal.addEventListener("abort", pending.onAbort, { once: true });
			this.#pending.set(request.id, pending);
			try {
				this.#send(request);
				if (signal.aborted) pending.onAbort();
			} catch (error) {
				this.#finishPending(request.id);
				resume(Effect.fail(brokerError(error)));
			}
			return Effect.sync(() => signal.removeEventListener("abort", pending.onAbort));
		});
	});

	/** Promise boundary adapter retained for Pi and existing callers. */
	exec(request: BrokerExecRequest, onData: (data: Buffer) => void, signal?: AbortSignal, onStarted?: (pid: number) => void): Promise<BrokerExecResult> {
		return Effect.runPromise(this.execEffect(request, onData, onStarted), signal ? { signal } : undefined);
	}

	writeStdin(id: string, data: Buffer): void {
		if (!this.#pending.get(id)?.started) throw new Error(`Command is not running: ${id}`);
		this.#send({ type: "write_stdin", id, data_base64: data.toString("base64") });
	}

	cancel(id: string): void {
		this.#cancelPending(id);
	}

	readonly shutdownEffect = Effect.fn("SandboxBrokerClient.shutdown")( () => {
		if (this.#exited || this.#shutdownStarted) return Effect.void;
		this.#shutdownStarted = true;
		const graceful = Effect.callback<void>((resume) => {
			const onClose = () => resume(Effect.void);
			this.#child.once("close", onClose);
			try {
				this.#send({ type: "shutdown" });
			} catch {
				this.#child.kill("SIGKILL");
			}
			return Effect.sync(() => this.#child.removeListener("close", onClose));
		});
		return Effect.raceFirst(
			graceful,
			Effect.sleep(SHUTDOWN_TIMEOUT_MS).pipe(Effect.andThen(Effect.sync(() => { if (!this.#exited) this.#child.kill("SIGKILL"); }))),
		);
	});

	/** Promise boundary adapter. */
	shutdown(): Promise<void> {
		const scope = this.#ownerScope;
		this.#ownerScope = undefined;
		return Effect.runPromise(scope ? Scope.close(scope, Exit.void) : this.shutdownEffect());
	}

	get #waitUntilReadyEffect(): Effect.Effect<void, SandboxBrokerError> {
		const ready = Effect.callback<void, SandboxBrokerError>((resume) => {
			if (this.#ready) return resume(Effect.void);
			if (this.#failed || this.#exited) return resume(Effect.fail(brokerError("Sandbox broker failed before readiness")));
			this.#readyResume = resume;
			return Effect.sync(() => { if (this.#readyResume === resume) this.#readyResume = undefined; });
		});
		return Effect.raceFirst(
			ready,
			Effect.sleep(READY_TIMEOUT_MS).pipe(Effect.andThen(Effect.fail(brokerError("Sandbox broker readiness timed out")))),
		);
	}

	#detachListeners(): void {
		this.#child.stdout.removeListener("data", this.#onStdout);
		this.#child.stderr.removeListener("data", this.#onStderr);
		this.#child.removeListener("error", this.#onError);
		this.#child.removeListener("close", this.#onClose);
		if (!this.#exited) this.#child.kill("SIGKILL");
	}

	#onChunk(chunk: Buffer): void {
		try {
			for (const message of this.#decoder.push(chunk)) this.#onEvent(validateBrokerEvent(message));
		} catch (error) {
			this.#fail(asError(error));
			this.#child.kill("SIGKILL");
		}
	}

	#onEvent(event: BrokerEvent): void {
		if (event.type === "ready") {
			if (this.#ready) throw new Error("Sandbox broker sent ready twice");
			if (!isSupportedReadyEvent(event, this.#expectedPlatform)) throw new Error(`Unsupported sandbox broker: version=${event.version}, platform=${event.platform}, backend=${event.backend}, can_exec=${event.can_exec}`);
			this.#requiresDenials = event.backend === "seatbelt";
			this.#ready = true;
			this.#readyResume?.(Effect.void);
			this.#readyResume = undefined;
			return;
		}
		if (!this.#ready) throw new Error("Sandbox broker sent an event before ready");
		if (event.type === "started") {
			const pending = this.#pending.get(event.id);
			if (!pending) throw new Error(`Broker start has unknown command ID: ${event.id}`);
			if (pending.started) throw new Error(`Broker started command twice: ${event.id}`);
			pending.started = true;
			pending.onStarted?.(event.pid);
			return;
		}
		if (event.type === "denials") {
			const pending = this.#pending.get(event.id);
			if (!pending?.started) throw new Error(`Broker denial has inactive command ID: ${event.id}`);
			if (pending.denials) throw new Error(`Broker sent denials twice: ${event.id}`);
			pending.denials = event.items;
			pending.denialsComplete = event.complete;
			return;
		}
		if (event.type === "stdout" || event.type === "stderr") {
			const pending = this.#pending.get(event.id);
			if (!pending?.started) throw new Error(`Broker output has inactive command ID: ${event.id}`);
			const expected = event.type === "stdout" ? pending.stdoutSequence : pending.stderrSequence;
			if (event.sequence !== expected) throw new Error(`Broker ${event.type} sequence mismatch for ${event.id}`);
			if (event.type === "stdout") pending.stdoutSequence += 1;
			else pending.stderrSequence += 1;
			pending.onData(Buffer.from(event.data_base64, "base64"));
			return;
		}
		if (event.type === "exit") {
			const pending = this.#pending.get(event.id);
			if (!pending?.started) throw new Error(`Broker exit has inactive command ID: ${event.id}`);
			if (this.#requiresDenials && !pending.denials) throw new Error(`Broker exit arrived before denials: ${event.id}`);
			this.#finishPending(event.id);
			if (event.output_truncated) pending.onData(Buffer.from("\n[Sandbox output truncated at the broker limit]\n"));
			if (pending.signal.aborted || event.cancelled) pending.resume(Effect.fail(brokerError("aborted")));
			else if (event.timed_out) pending.resume(Effect.fail(brokerError(`timeout:${pending.timeoutSeconds ?? "broker"}`)));
			else pending.resume(Effect.succeed({ exitCode: event.code ?? 1, denials: pending.denials ?? [], denialsComplete: pending.denialsComplete }));
			return;
		}
		if (event.id === null) throw new Error(`Sandbox broker error: ${event.message}`);
		const pending = this.#pending.get(event.id);
		if (!pending) throw new Error(`Broker error has unknown command ID: ${event.id}`);
		if (pending.started) throw new Error(`Broker sent a pre-start error after starting command: ${event.id}`);
		this.#finishPending(event.id);
		pending.resume(Effect.fail(brokerError(`Sandbox broker ${event.code}: ${event.message}`)));
	}

	#cancelPending(id: string): void {
		const pending = this.#pending.get(id);
		if (!pending || pending.cancelSent) return;
		pending.cancelSent = true;
		try {
			this.#send({ type: "cancel", id });
		} catch (error) {
			this.#fail(asError(error));
		}
	}

	#finishPending(id: string): PendingExec | undefined {
		const pending = this.#pending.get(id);
		if (!pending) return undefined;
		this.#pending.delete(id);
		pending.signal.removeEventListener("abort", pending.onAbort);
		return pending;
	}

	#send(message: BrokerRequest): void {
		if (!this.#child.stdin.writable) throw new Error("Sandbox broker input is closed");
		this.#child.stdin.write(encodeBrokerFrame(message));
	}

	#fail(error: Error): void {
		if (this.#failed) return;
		this.#failed = true;
		const failure = brokerError(error);
		this.#readyResume?.(Effect.fail(failure));
		this.#readyResume = undefined;
		for (const [id, pending] of this.#pending) {
			this.#finishPending(id);
			pending.resume(Effect.fail(failure));
		}
	}
}

export function isSupportedReadyEvent(event: Extract<BrokerEvent, { type: "ready" }>, platform: NodeJS.Platform): boolean {
	const expected = platform === "darwin"
		? { platform: "macos", backend: "seatbelt" }
		: platform === "linux" ? { platform: "linux", backend: "bubblewrap" } : undefined;
	return expected !== undefined && event.version === PROTOCOL_VERSION && event.platform === expected.platform && event.backend === expected.backend && event.can_exec && event.max_frame_bytes === MAX_BROKER_FRAME_BYTES;
}

export function validateBrokerEvent(value: unknown): BrokerEvent {
	if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Broker event is not an object");
	const event = value as Record<string, unknown>;
	if (typeof event.type !== "string") throw new Error("Broker event type is missing");
	if (event.type === "ready") {
		assertKeys(event, ["type", "version", "platform", "backend", "can_exec", "max_frame_bytes"]);
		assertInteger(event.version, "ready.version"); assertString(event.platform, "ready.platform"); assertString(event.backend, "ready.backend"); assertBoolean(event.can_exec, "ready.can_exec"); assertInteger(event.max_frame_bytes, "ready.max_frame_bytes");
	} else if (event.type === "started") {
		assertKeys(event, ["type", "id", "pid"]); assertString(event.id, "started.id"); assertInteger(event.pid, "started.pid");
	} else if (event.type === "stdout" || event.type === "stderr") {
		assertKeys(event, ["type", "id", "sequence", "data_base64"]); assertString(event.id, `${event.type}.id`); assertInteger(event.sequence, `${event.type}.sequence`); assertString(event.data_base64, `${event.type}.data_base64`);
		if (!isCanonicalBase64(event.data_base64)) throw new Error(`${event.type}.data_base64 is invalid`);
	} else if (event.type === "denials") {
		assertKeys(event, ["type", "id", "items", "complete"]); assertString(event.id, "denials.id"); assertBoolean(event.complete, "denials.complete");
		if (!Array.isArray(event.items) || event.items.length > 128) throw new Error("denials.items is invalid");
		for (const item of event.items) {
			if (!item || typeof item !== "object" || Array.isArray(item)) throw new Error("denial item is invalid");
			const denial = item as Record<string, unknown>; assertKeys(denial, ["operation", "path", "process"]); assertString(denial.operation, "denial.operation"); assertNullableString(denial.path, "denial.path"); assertNullableString(denial.process, "denial.process");
		}
	} else if (event.type === "exit") {
		assertKeys(event, ["type", "id", "code", "signal", "timed_out", "cancelled", "output_truncated"]); assertString(event.id, "exit.id"); assertNullableInteger(event.code, "exit.code"); assertNullableInteger(event.signal, "exit.signal"); assertBoolean(event.timed_out, "exit.timed_out"); assertBoolean(event.cancelled, "exit.cancelled"); assertBoolean(event.output_truncated, "exit.output_truncated");
	} else if (event.type === "error") {
		assertKeys(event, ["type", "id", "code", "message"]); assertNullableString(event.id, "error.id"); assertString(event.code, "error.code"); if (!BROKER_ERROR_CODES.has(event.code)) throw new Error("error.code is invalid"); assertString(event.message, "error.message");
	} else throw new Error(`Unknown broker event type: ${event.type}`);
	return value as BrokerEvent;
}

function assertKeys(record: Record<string, unknown>, expected: readonly string[]): void {
	const actual = Object.keys(record).sort(); const wanted = [...expected].sort();
	if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) throw new Error(`Broker event fields are invalid: ${actual.join(", ")}`);
}
function assertString(value: unknown, label: string): asserts value is string { if (typeof value !== "string" || value.includes("\0")) throw new Error(`${label} is invalid`); }
function assertNullableString(value: unknown, label: string): asserts value is string | null { if (value !== null) assertString(value, label); }
function assertInteger(value: unknown, label: string): asserts value is number { if (!Number.isSafeInteger(value) || (value as number) < 0) throw new Error(`${label} is invalid`); }
function assertNullableInteger(value: unknown, label: string): asserts value is number | null { if (value !== null && !Number.isSafeInteger(value)) throw new Error(`${label} is invalid`); }
function assertBoolean(value: unknown, label: string): asserts value is boolean { if (typeof value !== "boolean") throw new Error(`${label} is invalid`); }
function isCanonicalBase64(value: string): boolean {
	if (value.length % 4 !== 0 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) return false;
	return Buffer.from(value, "base64").toString("base64") === value;
}
function buildBrokerEnvironment(developmentCacheRoot?: string): NodeJS.ProcessEnv {
	const environment: NodeJS.ProcessEnv = {};
	for (const name of ["HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE"]) if (process.env[name] !== undefined) environment[name] = process.env[name];
	if (developmentCacheRoot !== undefined) {
		environment.PI_SANDBOX_DEVELOPMENT_CACHE_ROOT = developmentCacheRoot;
	}
	return environment;
}
function asError(error: unknown): Error { return error instanceof Error ? error : new Error(String(error)); }
