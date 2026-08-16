import type { BashOperations } from "@earendil-works/pi-coding-agent";
import { dirname } from "node:path";
import type {
	BrokerExecRequest,
	BrokerExecResult,
} from "./broker-client.ts";
import type { NativeSandboxConfig } from "./sandbox-config.ts";
import { buildBrokerExecRequest } from "./broker-policy.ts";
import {
	permissionForNativeDenial,
	type NativeFilePermission,
} from "./native-denials.ts";
import { parseFilesystemFailures } from "./sandbox-failures.ts";
import { startNativeNetworkProxy } from "./native-network-proxy.ts";

const MAX_NATIVE_ATTEMPTS = 8;
const MAX_FAILURE_OUTPUT_BYTES = 1024 * 1024;
const FOLDER_SIBLING_THRESHOLD = 4;
const NATIVE_RETRY_STARTED = "\n[Retrying command with approved IO rights]\n";
const NATIVE_RETRY_SUCCEEDED =
	"[Command completed successfully after approved IO retry]\n";

export interface NativeBroker {
	exec(
		request: BrokerExecRequest,
		onData: (data: Buffer) => void,
		signal?: AbortSignal,
	): Promise<BrokerExecResult>;
}

export interface NativeApprovalRequest {
	permissions: readonly NativeFilePermission[];
	folderAlternative?: NativeFilePermission;
}

export type NativeApprovalChoice =
	| "exact-once"
	| "exact-always"
	| "folder-once"
	| "folder-always"
	| "deny";

export interface NativeApprovalSelection {
	permissions: readonly NativeFilePermission[];
	persistent: boolean;
}

export function modelVisibleApprovedRetryOutput(output: string): string {
	const finalAttempt = output.lastIndexOf(NATIVE_RETRY_STARTED);
	return finalAttempt >= 0 ? output.slice(finalAttempt) : output;
}

export function resolveNativeApprovalChoice(
	request: NativeApprovalRequest,
	choice: NativeApprovalChoice,
): NativeApprovalSelection | undefined {
	if (choice === "deny") return undefined;
	if (choice === "exact-once" || choice === "exact-always") {
		return {
			permissions: request.permissions,
			persistent: choice === "exact-always",
		};
	}
	if (!request.folderAlternative) return undefined;
	return {
		permissions: [request.folderAlternative],
		persistent: choice === "folder-always",
	};
}

export function createNativeSandboxOps(
	client: NativeBroker,
	config: NativeSandboxConfig,
	permissions: readonly NativeFilePermission[],
	networkHosts: readonly string[],
	commandId: string,
	allowLocalBinding = false,
): BashOperations {
	return {
		async exec(command, cwd, { onData, signal, timeout }) {
			const proxy = networkHosts.length > 0
				? await startNativeNetworkProxy(networkHosts)
				: undefined;
			try {
				const request = buildBrokerExecRequest(
					commandId,
					command,
					cwd,
					timeout,
					config,
					permissions,
					networkHosts,
					proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
					allowLocalBinding,
				);
				return await client.exec(request, onData, signal);
			} finally {
				await proxy?.close();
			}
		},
	};
}

export function createApprovingNativeSandboxOps(options: {
	client: NativeBroker;
	config: NativeSandboxConfig;
	initialPermissions: readonly NativeFilePermission[];
	initialNetworkHosts?: readonly string[];
	initialAllowLocalBinding?: boolean;
	toolCallId: string;
	blockedPaths: readonly string[];
	approve: (
		request: NativeApprovalRequest,
		signal: AbortSignal | undefined,
	) => Promise<readonly NativeFilePermission[] | undefined>;
}): BashOperations {
	return {
		async exec(command, cwd, execOptions) {
			const networkHosts = options.initialNetworkHosts ?? [];
			const allowLocalBinding = options.initialAllowLocalBinding ?? false;
			const proxy = networkHosts.length > 0
				? await startNativeNetworkProxy(networkHosts)
				: undefined;
			const permissions = [...options.initialPermissions];
			const denialHistory = new Map<string, Map<string, NativeFilePermission>>();
			let lastResult: BrokerExecResult = {
				exitCode: 1,
				denials: [],
				denialsComplete: false,
			};

			try {
				for (let attempt = 0; attempt < MAX_NATIVE_ATTEMPTS; attempt += 1) {
				const output: Buffer[] = [];
				let retainedBytes = 0;
				const request = buildBrokerExecRequest(
					`${options.toolCallId}/attempt-${attempt}`,
					command,
					cwd,
					execOptions.timeout,
					options.config,
					permissions,
					networkHosts,
					proxy ? { port: proxy.port, socketPath: proxy.socketPath } : undefined,
					allowLocalBinding,
				);
				lastResult = await options.client.exec(
					request,
					(data) => {
						if (retainedBytes < MAX_FAILURE_OUTPUT_BYTES) {
							const retained = data.subarray(
								0,
								MAX_FAILURE_OUTPUT_BYTES - retainedBytes,
							);
							output.push(retained);
							retainedBytes += retained.length;
						}
						execOptions.onData(data);
					},
					execOptions.signal,
				);
				if (execOptions.signal?.aborted) throw new Error("aborted");
				if (lastResult.exitCode === 0) {
					if (attempt > 0) {
						execOptions.onData(Buffer.from(NATIVE_RETRY_SUCCEEDED));
					}
					return lastResult;
				}

				const requested = new Map<string, NativeFilePermission>();
				const denials =
					lastResult.denials.length > 0
						? lastResult.denials
						: parseFilesystemFailures(
								Buffer.concat(output).toString("utf8"),
							).map((failure) => ({
								operation:
									failure.targetType === "folder"
										? "file-write-create-directory"
										: "file-write-create",
								path: failure.path,
								process: "stderr-fallback",
							}));
				for (const denial of denials) {
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
						requested.set(permissionKey(permission), permission);
					}
				}
				if (requested.size === 0) return lastResult;
				if (attempt + 1 >= MAX_NATIVE_ATTEMPTS) return lastResult;

				for (const permission of requested.values()) {
					if (permission.directory) continue;
					const key = siblingGroupKey(permission);
					const group = denialHistory.get(key) ?? new Map<string, NativeFilePermission>();
					group.set(permission.path, permission);
					denialHistory.set(key, group);
				}

				const approvalRequests: NativeApprovalRequest[] = [];
				const grouped = new Set<string>();
				for (const permission of requested.values()) {
					if (!permission.directory) {
						const groupKey = siblingGroupKey(permission);
						const siblings = denialHistory.get(groupKey);
						if (
							siblings &&
							siblings.size >= FOLDER_SIBLING_THRESHOLD &&
							!grouped.has(groupKey)
						) {
							grouped.add(groupKey);
							const folderDecision = permissionForNativeDenial(
								{
									operation:
										permission.kind === "write"
											? "file-write-create"
											: "file-read-metadata",
									path: dirname(permission.path),
									process: "grouped-denials",
								},
								cwd,
								options.config,
								permissions,
								options.blockedPaths,
							);
							approvalRequests.push({
								permissions: [...siblings.values()],
								folderAlternative:
									folderDecision.kind === "permission" &&
									folderDecision.permission.directory
										? folderDecision.permission
										: undefined,
							});
							continue;
						}
						if (grouped.has(groupKey)) continue;
					}
					approvalRequests.push({ permissions: [permission] });
				}

				for (const approvalRequest of approvalRequests) {
					const approved = await options.approve(approvalRequest, execOptions.signal);
					if (execOptions.signal?.aborted) throw new Error("aborted");
					if (!approved || approved.length === 0) return lastResult;
					for (const permission of approved) {
						if (!permissions.some((entry) => permissionKey(entry) === permissionKey(permission))) {
							permissions.push(permission);
						}
					}
				}
				if (execOptions.signal?.aborted) throw new Error("aborted");
				execOptions.onData(Buffer.from(NATIVE_RETRY_STARTED));
				}
				return lastResult;
			} finally {
				await proxy?.close();
			}
		},
	};
}

function permissionKey(permission: NativeFilePermission): string {
	return `${permission.kind}:${permission.directory}:${permission.path}`;
}

function siblingGroupKey(permission: NativeFilePermission): string {
	return `${permission.kind}:${dirname(permission.path)}`;
}
