import {
	SandboxBrokerClient,
	type BrokerExecRequest,
	type BrokerExecResult,
} from "./broker-client.ts";
import {
	buildBrokerExecRequest,
	type NativeFilePermission,
} from "./broker-policy.ts";
import { formatDenialSummary } from "./denial-summary.ts";
import type { NativeSandboxConfig } from "./sandbox-config.ts";
import { startNativeNetworkProxy, type NativeNetworkProxy } from "./native-network-proxy.ts";

const MAX_RETAINED_BYTES = 2 * 1024 * 1024;
const MAX_JOBS = 32;

interface NativeJob {
	name: string;
	client: SandboxBrokerClient;
	proxy?: NativeNetworkProxy;
	output: Buffer;
	startedAt: Date;
	pid?: number;
	result?: BrokerExecResult;
	error?: string;
	done: Promise<void>;
}

export class NativeBackgroundJobs {
	readonly #brokerPath: string;
	readonly #jobs = new Map<string, NativeJob>();

	constructor(brokerPath: string) {
		this.#brokerPath = brokerPath;
	}

	async start(options: {
		name: string;
		command: string;
		cwd: string;
		config: NativeSandboxConfig;
		permissions: readonly NativeFilePermission[];
		revalidatePermissions?: () => readonly NativeFilePermission[];
		networkHosts: readonly string[];
		allowLocalBinding?: boolean;
	}): Promise<string> {
		if (this.#jobs.has(options.name)) throw new Error(`job already exists: ${options.name}`);
		if (this.#jobs.size >= MAX_JOBS) {
			throw new Error(`background job limit reached: ${MAX_JOBS}`);
		}
		const client = await SandboxBrokerClient.start(this.#brokerPath);
		let proxy: NativeNetworkProxy | undefined;
		try {
			proxy = options.networkHosts.length > 0
				? await startNativeNetworkProxy(options.networkHosts)
				: undefined;
		} catch (error) {
			await client.shutdown();
			throw error;
		}
		let startedResolve!: () => void;
		let startedReject!: (error: Error) => void;
		const started = new Promise<void>((resolve, reject) => {
			startedResolve = resolve;
			startedReject = reject;
		});
		let request: BrokerExecRequest;
		try {
			const currentPermissions = options.revalidatePermissions?.() ?? options.permissions;
			request = buildBrokerExecRequest(
				`background/${options.name}`,
				options.command,
				options.cwd,
				undefined,
				options.config,
				currentPermissions,
				options.networkHosts,
				proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
				options.allowLocalBinding ?? false,
			);
		} catch (error) {
			await proxy?.close();
			await client.shutdown();
			throw error;
		}
		request.interactive = true;
		const job: NativeJob = {
			name: options.name,
			client,
			proxy,
			output: Buffer.alloc(0),
			startedAt: new Date(),
			done: Promise.resolve(),
		};
		this.#jobs.set(options.name, job);
		job.done = client
			.exec(
				request,
				(data) => {
					job.output = Buffer.concat([job.output, data]);
					if (job.output.length > MAX_RETAINED_BYTES) {
						job.output = job.output.subarray(job.output.length - MAX_RETAINED_BYTES);
					}
				},
				undefined,
				(pid) => {
					job.pid = pid;
					startedResolve();
				},
			)
			.then((result) => {
				job.result = result;
				if (result.exitCode !== 0) {
					const summary = formatDenialSummary(result.denials, result.denialsComplete);
					if (summary) {
						job.output = Buffer.concat([job.output, Buffer.from(summary)]);
						if (job.output.length > MAX_RETAINED_BYTES) {
							job.output = job.output.subarray(job.output.length - MAX_RETAINED_BYTES);
						}
					}
				}
			})
			.catch((error: unknown) => {
				job.error = error instanceof Error ? error.message : String(error);
				startedReject(error instanceof Error ? error : new Error(String(error)));
			})
			.finally(async () => {
				await job.proxy?.close();
				await job.client.shutdown();
			});
		try {
			await started;
		} catch (error) {
			this.#jobs.delete(options.name);
			await job.done;
			throw error;
		}
		return `started ${options.name}`;
	}

	list(): string {
		if (this.#jobs.size === 0) return "no background jobs";
		return [...this.#jobs.values()]
			.map((job) => `${job.name} ${jobState(job)} pid=${job.pid ?? "unknown"} started=${job.startedAt.toISOString()}`)
			.join("\n");
	}

	status(name: string): string {
		const job = this.#require(name);
		return `name=${name} state=${jobState(job)} pid=${job.pid ?? "unknown"}${
			job.result ? ` exit=${job.result.exitCode ?? 1}` : ""
		}${job.error ? ` error=${job.error}` : ""}`;
	}

	read(name: string, lines: number): string {
		const job = this.#require(name);
		return job.output.toString("utf8").split("\n").slice(-lines).join("\n");
	}

	write(name: string, data: Buffer): string {
		const job = this.#requireRunning(name);
		job.client.writeStdin(`background/${name}`, data);
		return `sent input to ${name}`;
	}

	async stop(name: string): Promise<string> {
		const job = this.#require(name);
		if (!job.result && !job.error) job.client.cancel(`background/${name}`);
		await job.done;
		this.#jobs.delete(name);
		return `stopped ${name}`;
	}

	async shutdown(): Promise<void> {
		await Promise.all([...this.#jobs.keys()].map((name) => this.stop(name).catch(() => undefined)));
	}

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

function jobState(job: NativeJob): string {
	if (job.error) return "failed";
	if (job.result) return job.result.exitCode === 0 ? "completed" : "exited";
	return "running";
}

export function backgroundKeyBytes(keys: readonly string[]): Buffer {
	const values: Record<string, string> = {
		Enter: "\n",
		Tab: "\t",
		BSpace: "\x7f",
		Escape: "\x1b",
		"C-c": "\x03",
		"C-d": "\x04",
		"C-z": "\x1a",
	};
	return Buffer.from(keys.map((key) => values[key] ?? key).join(""));
}
