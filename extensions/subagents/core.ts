import { Effect, Fiber } from "effect";
import type {
	ChildSession,
	ChildSessionFactory,
	RunSnapshot,
	SendMode,
	StartRequest,
} from "./contract.ts";

interface RunRecord {
	session: ChildSession;
	snapshot: RunSnapshot;
	fiber?: Fiber.Fiber<void, never>;
	stopping: boolean;
	disposed: boolean;
}

function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function terminal(snapshot: RunSnapshot): boolean {
	return snapshot.status !== "running";
}

export class SubagentCore {
	readonly #runs = new Map<string, RunRecord>();
	readonly #factory: ChildSessionFactory;
	readonly #onSettled: (snapshot: RunSnapshot) => void;

	constructor(factory: ChildSessionFactory, onSettled: (snapshot: RunSnapshot) => void = () => {}) {
		this.#factory = factory;
		this.#onSettled = onSettled;
	}

	start(request: StartRequest) {
		const self = this;
		return Effect.gen(function* () {
			const context = request.context ?? "fork";
			if (context === "fork" && !request.parentSessionFile) {
				return yield* Effect.fail(new Error("Cannot fork: the parent session is not persistent"));
			}

			const session = yield* Effect.tryPromise({
				try: () => self.#factory.create({ ...request, context }),
				catch: (error) => new Error(`Failed to create subagent session: ${errorMessage(error)}`),
			});
			if (self.#runs.has(session.id)) {
				yield* Effect.promise(() => Promise.resolve(session.dispose()));
				return yield* Effect.fail(new Error(`Duplicate subagent session id: ${session.id}`));
			}

			const record: RunRecord = {
				session,
				snapshot: {
					id: session.id,
					sessionId: session.id,
					sessionFile: session.sessionFile,
					status: "running",
					context,
					provider: session.provider,
					model: session.model,
					effort: session.effort,
				},
				stopping: false,
				disposed: false,
			};
			self.#runs.set(session.id, record);
			self.#launch(record, () => session.run(request.prompt));
			return self.#copy(record.snapshot);
		});
	}

	send(id: string, message: string, requestedMode?: SendMode) {
		const self = this;
		return Effect.gen(function* () {
			const record = yield* self.#requireUsable(id);
			const running = record.session.isStreaming();
			const mode = requestedMode ?? (running ? "steer" : "prompt");
			if (mode === "steer" && !running) {
				return yield* Effect.fail(new Error(`Cannot steer idle subagent ${id}; send a prompt instead`));
			}
			if (terminal(record.snapshot)) {
				const { output: _output, error: _error, ...snapshot } = record.snapshot;
				record.snapshot = { ...snapshot, status: "running" };
				self.#launch(record, () => record.session.run(message));
			} else {
				yield* Effect.tryPromise({
					try: () => record.session.send(message, mode),
					catch: (error) => new Error(`Failed to send to ${id}: ${errorMessage(error)}`),
				});
			}
			return { id, mode, status: record.snapshot.status } as const;
		});
	}

	control(action: "list", id?: string): Effect.Effect<RunSnapshot[], Error>;
	control(action: "status" | "stop", id?: string): Effect.Effect<RunSnapshot, Error>;
	control(action: "list" | "status" | "stop", id?: string): Effect.Effect<RunSnapshot | RunSnapshot[], Error>;
	control(action: "list" | "status" | "stop", id?: string): Effect.Effect<RunSnapshot | RunSnapshot[], Error> {
		switch (action) {
			case "list":
				return Effect.sync(() => Array.from(this.#runs.values(), (record) => this.#copy(record.snapshot)));
			case "status":
				return Effect.try({
					try: () => this.#copy(this.#requireId(id).snapshot),
					catch: (error) => error instanceof Error ? error : new Error(String(error)),
				});
			case "stop": {
				const self = this;
				return Effect.gen(function* () {
					const record = yield* Effect.try({
						try: () => self.#requireId(id),
						catch: (error) => error instanceof Error ? error : new Error(String(error)),
					});
					if (!terminal(record.snapshot)) {
						record.stopping = true;
						yield* Effect.tryPromise({
							try: () => record.session.abort(),
							catch: (error) => new Error(`Failed to stop ${record.snapshot.id}: ${errorMessage(error)}`),
						});
						yield* self.#finish(record, "stopped");
						yield* self.#dispose(record);
					}
					return self.#copy(record.snapshot);
				});
			}
		}
	}

	shutdown() {
		return Effect.forEach(this.#runs.values(), (record) => {
			const stop = terminal(record.snapshot)
				? Effect.void
				: Effect.tryPromise(() => record.session.abort()).pipe(
					Effect.ignore,
					Effect.andThen(this.#finish(record, "stopped", undefined, undefined, false)),
				);
			return stop.pipe(Effect.andThen(this.#dispose(record)), Effect.ignore);
		}, { discard: true });
	}

	#dispose(record: RunRecord) {
		if (record.disposed) return Effect.void;
		record.disposed = true;
		return Effect.promise(() => Promise.resolve(record.session.dispose()));
	}

	#launch(record: RunRecord, run: () => Promise<string>): void {
		const task = Effect.tryPromise({ try: run, catch: (error) => error }).pipe(
			Effect.matchEffect({
				onFailure: (error) => record.stopping
					? this.#finish(record, "stopped")
					: this.#finish(record, "failed", undefined, errorMessage(error)),
				onSuccess: (output) => this.#finish(record, "idle", output),
			}),
			Effect.catch(() => Effect.void),
		);
		record.fiber = Effect.runFork(task);
	}

	#finish(
		record: RunRecord,
		status: "idle" | "failed" | "stopped",
		output?: string,
		error?: string,
		notify = true,
	) {
		return Effect.sync(() => {
			if (terminal(record.snapshot)) return;
			record.snapshot = {
				...record.snapshot,
				status,
				...(output === undefined ? {} : { output }),
				...(error === undefined ? {} : { error }),
			};
			if (notify && status !== "stopped") this.#onSettled(this.#copy(record.snapshot));
		});
	}

	#require(id: string): RunRecord {
		const record = this.#runs.get(id);
		if (!record) throw new Error(`Unknown subagent id: ${id}`);
		return record;
	}

	#requireId(id?: string): RunRecord {
		if (!id) throw new Error("id is required for this action");
		return this.#require(id);
	}

	#requireUsable(id: string) {
		return Effect.try({
			try: () => {
				const record = this.#require(id);
				if (record.snapshot.status === "stopped") throw new Error(`Subagent ${id} is stopped`);
				return record;
			},
			catch: (error) => error instanceof Error ? error : new Error(String(error)),
		});
	}

	#copy(snapshot: RunSnapshot): RunSnapshot {
		return { ...snapshot };
	}
}
