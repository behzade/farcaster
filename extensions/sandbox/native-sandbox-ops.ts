import type { BashOperations } from "@earendil-works/pi-coding-agent";
import { Effect, Schema } from "effect";
import type {
	BrokerExecRequest,
	BrokerExecResult,
} from "./broker-client.ts";
import type { NativeSandboxConfig } from "./sandbox-config.ts";
import {
	buildBrokerExecRequest,
	type NativeFilePermission,
} from "./broker-policy.ts";
import { formatDenialSummary } from "./denial-summary.ts";
import { acquireNativeNetworkProxy, type NativeNetworkProxy } from "./native-network-proxy.ts";

export interface NativeBroker {
	exec(
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
		signal?: AbortSignal,
	): Promise<BrokerExecResult>;
	execEffect?: (
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
	) => Effect.Effect<BrokerExecResult, unknown>;
}

export class NativeSandboxExecError extends Schema.TaggedError<NativeSandboxExecError>()(
	"NativeSandboxExecError",
	{
		message: Schema.String,
		cause: Schema.optional(Schema.Defect()),
	},
) {}

const sandboxError = (cause: unknown) => new NativeSandboxExecError({
	message: cause instanceof Error ? cause.message : String(cause),
	cause,
});

const acquireNetworkProxy = (networkHosts: readonly string[]) =>
	networkHosts.length === 0
		? Effect.succeed(undefined)
		: acquireNativeNetworkProxy(networkHosts).pipe(Effect.mapError(sandboxError));

export const executeNativeSandboxCommand = Effect.fn("Sandbox.executeNativeCommand")(
	function* (params: {
		client: NativeBroker;
		config: NativeSandboxConfig;
		permissions: readonly NativeFilePermission[];
		networkHosts: readonly string[];
		commandId: string;
		allowLocalBinding: boolean;
		revalidatePermissions?: () => readonly NativeFilePermission[];
		command: string;
		cwd: string;
		onData: (data: Buffer) => void;
		signal?: AbortSignal;
		timeout?: number;
	}) {
		const proxy: NativeNetworkProxy | undefined = yield* acquireNetworkProxy(params.networkHosts);
		const request = yield* Effect.try({
			try: () => buildBrokerExecRequest(
				params.commandId,
				params.command,
				params.cwd,
				params.timeout,
				params.config,
				params.revalidatePermissions?.() ?? params.permissions,
				params.networkHosts,
				proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
				params.allowLocalBinding,
			),
			catch: sandboxError,
		});
		const result = yield* (params.client.execEffect
			? params.client.execEffect(request, params.onData).pipe(Effect.mapError(sandboxError))
			: Effect.tryPromise({
				try: (effectSignal) => params.client.exec(request, params.onData, params.signal ?? effectSignal),
				catch: sandboxError,
			}));
		if (result.exitCode !== 0) {
			const summary = formatDenialSummary(result.denials, result.denialsComplete);
			if (summary) yield* Effect.sync(() => params.onData(Buffer.from(summary)));
		}
		return result;
	},
);

/** Executes exactly once. Access changes are separate request_access tool calls. */
export function createNativeSandboxOps(
	client: NativeBroker,
	config: NativeSandboxConfig,
	permissions: readonly NativeFilePermission[],
	networkHosts: readonly string[],
	commandId: string,
	allowLocalBinding = false,
	revalidatePermissions?: () => readonly NativeFilePermission[],
): BashOperations {
	return {
		exec(command, cwd, { onData, signal, timeout }) {
			return Effect.runPromise(Effect.scoped(executeNativeSandboxCommand({
				client,
				config,
				permissions,
				networkHosts,
				commandId,
				allowLocalBinding,
				revalidatePermissions,
				command,
				cwd,
				onData,
				signal,
				timeout,
			})), { signal });
		},
	};
}
