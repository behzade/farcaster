import type { BashOperations } from "@earendil-works/pi-coding-agent";
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
import { startNativeNetworkProxy } from "./native-network-proxy.ts";

export interface NativeBroker {
	exec(
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
		signal?: AbortSignal,
	): Promise<BrokerExecResult>;
}

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
		async exec(command, cwd, { onData, signal, timeout }) {
			const proxy = networkHosts.length > 0
				? await startNativeNetworkProxy(networkHosts)
				: undefined;
			try {
				const currentPermissions = revalidatePermissions?.() ?? permissions;
				const request = buildBrokerExecRequest(
					commandId,
					command,
					cwd,
					timeout,
					config,
					currentPermissions,
					networkHosts,
					proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
					allowLocalBinding,
				);
				const result = await client.exec(request, onData, signal);
				if (result.exitCode !== 0) {
					const summary = formatDenialSummary(result.denials, result.denialsComplete);
					if (summary) onData(Buffer.from(summary));
				}
				return result;
			} finally {
				await proxy?.close();
			}
		},
	};
}
