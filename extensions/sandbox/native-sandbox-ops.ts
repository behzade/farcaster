import type { BashOperations } from "@earendil-works/pi-coding-agent";
import type {
	BrokerExecRequest,
	BrokerExecResult,
} from "./broker-client.ts";
import type { CodexSandboxConfig } from "./codex-command.ts";
import { buildBrokerExecRequest } from "./broker-policy.ts";
import {
	permissionForNativeDenial,
	type NativeFilePermission,
} from "./native-denials.ts";

const MAX_NATIVE_ATTEMPTS = 4;

export interface NativeBroker {
	exec(
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
		signal?: AbortSignal,
	): Promise<BrokerExecResult>;
}

export function createNativeSandboxOps(
	client: NativeBroker,
	config: CodexSandboxConfig,
	permissions: readonly NativeFilePermission[],
	networkHosts: readonly string[],
	commandId: string,
): BashOperations {
	return {
		async exec(command, cwd, { onData, signal, timeout }) {
			const request = buildBrokerExecRequest(
				commandId,
				command,
				cwd,
				timeout,
				config,
				permissions,
				networkHosts,
			);
			return client.exec(request, onData, signal);
		},
	};
}

export function createApprovingNativeSandboxOps(options: {
	client: NativeBroker;
	config: CodexSandboxConfig;
	initialPermissions: readonly NativeFilePermission[];
	toolCallId: string;
	blockedPaths: readonly string[];
	approve: (
		permission: NativeFilePermission,
		signal: AbortSignal | undefined,
	) => Promise<boolean>;
}): BashOperations {
	return {
		async exec(command, cwd, execOptions) {
			const permissions = [...options.initialPermissions];
			let lastResult: BrokerExecResult = {
				exitCode: 1,
				denials: [],
				denialsComplete: false,
			};

			for (let attempt = 0; attempt < MAX_NATIVE_ATTEMPTS; attempt += 1) {
				const request = buildBrokerExecRequest(
					`${options.toolCallId}/attempt-${attempt}`,
					command,
					cwd,
					execOptions.timeout,
					options.config,
					permissions,
					[],
				);
				lastResult = await options.client.exec(
					request,
					execOptions.onData,
					execOptions.signal,
				);
				if (execOptions.signal?.aborted) throw new Error("aborted");
				if (lastResult.exitCode === 0) return lastResult;

				const requested = new Map<string, NativeFilePermission>();
				for (const denial of lastResult.denials) {
					const decision = permissionForNativeDenial(
						denial,
						cwd,
						options.config,
						permissions,
						options.blockedPaths,
					);
					if (decision.kind === "unsafe") return lastResult;
					if (decision.kind === "permission") {
						const permission = decision.permission;
						requested.set(
							`${permission.kind}:${permission.directory}:${permission.path}`,
							permission,
						);
					}
				}
				if (requested.size === 0) return lastResult;

				for (const permission of requested.values()) {
					const approved = await options.approve(permission, execOptions.signal);
					if (execOptions.signal?.aborted) throw new Error("aborted");
					if (!approved) return lastResult;
					permissions.push(permission);
				}
				if (execOptions.signal?.aborted) throw new Error("aborted");
				execOptions.onData(Buffer.from("\n[Retrying command with approved IO rights]\n"));
			}
			return lastResult;
		},
	};
}
