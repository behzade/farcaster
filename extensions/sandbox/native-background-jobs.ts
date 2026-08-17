import { Cause, Deferred, Effect, Exit, Fiber, Schema, Scope } from "effect";
import {
	SandboxBrokerClient,
	type BrokerExecRequest,
	type BrokerExecResult,
} from "./broker-client.ts";
import { buildBrokerExecRequest, type NativeFilePermission } from "./broker-policy.ts";
import { developmentCacheWriteRightsForWorkspace } from "./development-caches.ts";
import { formatDenialSummary } from "./denial-summary.ts";
import type { NativeSandboxConfig } from "./sandbox-config.ts";
import { acquireNativeNetworkProxy, type NativeNetworkProxy } from "./native-network-proxy.ts";

const MAX_RETAINED_BYTES = 2 * 1024 * 1024;
const MAX_JOBS = 32;

export class NativeBackgroundJobError extends Schema.TaggedError<NativeBackgroundJobError>()(
	"NativeBackgroundJobError",
	{ message: Schema.String, cause: Schema.optional(Schema.Defect()) },
) {}

const jobError = (cause: unknown) => new NativeBackgroundJobError({
	message: cause instanceof Error ? cause.message : String(cause),
	cause,
});

interface NativeJob {
	name: string;
	client: SandboxBrokerClient;
	proxy?: NativeNetworkProxy;
	output: Buffer;
	startedAt: Date;
	pid?: number;
	result?: BrokerExecResult;
	error?: string;
	fiber: Fiber.Fiber<void, never>;
	scope: Scope.Closeable;
}

interface StartOptions {
	name: string;
	command: string;
	cwd: string;
	config: NativeSandboxConfig;
	permissions: readonly NativeFilePermission[];
	revalidatePermissions?: () => readonly NativeFilePermission[];
	networkHosts: readonly string[];
	allowLocalBinding?: boolean;
}

export class NativeBackgroundJobs {
	readonly #brokerPath: string;
	readonly #jobs = new Map<string, NativeJob>();
	readonly #scope = Scope.makeUnsafe();
	#closed = false;

	constructor(brokerPath: string) { this.#brokerPath = brokerPath; }

	readonly startEffect = Effect.fn("NativeBackgroundJobs.start")(function* (this: NativeBackgroundJobs, options: StartOptions) {
		if (this.#closed) return yield* Effect.fail(jobError("background jobs are shut down"));
		if (this.#jobs.has(options.name)) return yield* Effect.fail(jobError(`job already exists: ${options.name}`));
		if (this.#jobs.size >= MAX_JOBS) return yield* Effect.fail(jobError(`background job limit reached: ${MAX_JOBS}`));

		const manager = this;
		const jobScope = yield* Scope.make();
		const acquire = Effect.gen(function* () {
			const cacheRoot = developmentCacheWriteRightsForWorkspace(
				options.cwd,
				options.config.developmentCache,
			)[0]?.path;
			const client = yield* SandboxBrokerClient.acquire(
				manager.#brokerPath,
				process.platform,
				cacheRoot,
			).pipe(Scope.provide(jobScope));
			const proxy = options.networkHosts.length > 0
				? yield* acquireNativeNetworkProxy(options.networkHosts).pipe(Scope.provide(jobScope))
				: undefined;
			const request = yield* Effect.try({
				try: () => buildBrokerExecRequest(
					`background/${options.name}`,
					options.command,
					options.cwd,
					undefined,
					options.config,
					options.revalidatePermissions?.() ?? options.permissions,
					options.networkHosts,
					proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
					options.allowLocalBinding ?? false,
				),
				catch: jobError,
			});
			request.interactive = true;
			return { client, proxy, request };
		}).pipe(Effect.onExit((exit) => Exit.isFailure(exit) ? Scope.close(jobScope, exit) : Effect.void));
		const { client, proxy, request } = yield* acquire;
		const started = yield* Deferred.make<void, NativeBackgroundJobError>();
		const job = {
			name: options.name,
			client,
			proxy,
			output: Buffer.alloc(0),
			startedAt: new Date(),
			scope: jobScope,
		} as NativeJob;

		const run = Effect.gen(function* () {
			const exit = yield* Effect.exit(client.execEffect(
				request,
				(data) => appendOutput(job, data),
				(pid) => {
					job.pid = pid;
					Deferred.doneUnsafe(started, Effect.void);
				},
			));
			if (Exit.isSuccess(exit)) {
				job.result = exit.value;
				if (exit.value.exitCode !== 0) {
					const summary = formatDenialSummary(exit.value.denials, exit.value.denialsComplete);
					if (summary) appendOutput(job, Buffer.from(summary));
				}
			} else {
				const cause = Cause.squash(exit.cause);
				job.error = cause instanceof Error ? cause.message : String(cause);
				Deferred.doneUnsafe(started, Effect.fail(jobError(cause)));
			}
		}).pipe(
			Effect.onExit((exit) => Effect.sync(() => {
				if (!Deferred.isDoneUnsafe(started)) Deferred.doneUnsafe(started, Effect.fail(jobError("background job ended before starting")));
			}).pipe(Effect.andThen(Scope.close(jobScope, exit)))),
			Effect.catchCause(() => Effect.void),
		);
		job.fiber = yield* Effect.forkIn(run, this.#scope);
		this.#jobs.set(options.name, job);
		yield* Deferred.await(started).pipe(Effect.onExit((exit) => {
			if (Exit.isSuccess(exit)) return Effect.void;
			this.#jobs.delete(options.name);
			return Fiber.interrupt(job.fiber);
		}));
		return `started ${options.name}`;
	});

	/** Promise boundary adapter. */
	start(options: StartOptions, signal?: AbortSignal): Promise<string> {
		return Effect.runPromise(this.startEffect(options), signal ? { signal } : undefined);
	}

	list(): string {
		if (this.#jobs.size === 0) return "no background jobs";
		return [...this.#jobs.values()].map((job) => `${job.name} ${jobState(job)} pid=${job.pid ?? "unknown"} started=${job.startedAt.toISOString()}`).join("\n");
	}

	status(name: string): string {
		const job = this.#require(name);
		return `name=${name} state=${jobState(job)} pid=${job.pid ?? "unknown"}${job.result ? ` exit=${job.result.exitCode ?? 1}` : ""}${job.error ? ` error=${job.error}` : ""}`;
	}

	read(name: string, lines: number): string { return this.#require(name).output.toString("utf8").split("\n").slice(-lines).join("\n"); }

	write(name: string, data: Buffer): string {
		const job = this.#requireRunning(name);
		job.client.writeStdin(`background/${name}`, data);
		return `sent input to ${name}`;
	}

	readonly stopEffect = Effect.fn("NativeBackgroundJobs.stop")(function* (this: NativeBackgroundJobs, name: string) {
		const job = this.#require(name);
		if (!job.result && !job.error) yield* Fiber.interrupt(job.fiber);
		else yield* Fiber.await(job.fiber);
		this.#jobs.delete(name);
		return `stopped ${name}`;
	});

	/** Promise boundary adapter. */
	stop(name: string): Promise<string> { return Effect.runPromise(this.stopEffect(name)); }

	readonly shutdownEffect = Effect.fn("NativeBackgroundJobs.shutdown")(function* (this: NativeBackgroundJobs) {
		if (this.#closed) return;
		this.#closed = true;
		for (const job of this.#jobs.values()) yield* Fiber.interrupt(job.fiber);
		this.#jobs.clear();
		yield* Scope.close(this.#scope, Exit.void);
	});

	/** Promise boundary adapter. */
	shutdown(): Promise<void> { return Effect.runPromise(this.shutdownEffect()); }

	#require(name: string): NativeJob {
		const job = this.#jobs.get(name);
		if (!job) throw new Error(`unknown background job: ${name}`);
		return job;
	}
	#requireRunning(name: string): NativeJob {
		const job = this.#require(name);
		if (job.result || job.error) throw new Error(`background job is not running: ${name}`);
		return job;
	}
}

function appendOutput(job: NativeJob, data: Buffer): void {
	job.output = Buffer.concat([job.output, data]);
	if (job.output.length > MAX_RETAINED_BYTES) job.output = job.output.subarray(job.output.length - MAX_RETAINED_BYTES);
}
function jobState(job: NativeJob): string {
	if (job.error) return "failed";
	if (job.result) return job.result.exitCode === 0 ? "completed" : "exited";
	return "running";
}

export function backgroundKeyBytes(keys: readonly string[]): Buffer {
	const values: Record<string, string> = { Enter: "\n", Tab: "\t", BSpace: "\x7f", Escape: "\x1b", "C-c": "\x03", "C-d": "\x04", "C-z": "\x1a" };
	return Buffer.from(keys.map((key) => values[key] ?? key).join(""));
}
