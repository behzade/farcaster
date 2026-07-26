import type { DebugLogger, PermissionsState } from "../types.js";
import { readPermissionsFile, getGlobalPermissionsPath, getWorkspacePermissionsPath } from "./persistence.js";
import { buildPermissionRules, buildPolicyOverrides, buildPolicyRules } from "./matching.js";

export function loadPermissionsState(
	cwd: string,
	logDebug: DebugLogger,
	projectTrusted: boolean,
): PermissionsState {
	const workspacePath = getWorkspacePermissionsPath(cwd);
	const globalPath = getGlobalPermissionsPath();
	const workspaceSettings = projectTrusted ? readPermissionsFile(workspacePath, logDebug) : {};
	const globalSettings = readPermissionsFile(globalPath, logDebug);

	const workspaceRules = buildPermissionRules(workspaceSettings, "workspace", workspacePath, logDebug);
	const globalRules = buildPermissionRules(globalSettings, "global", globalPath, logDebug);
	const workspacePolicyRules = buildPolicyRules(workspaceSettings, "workspace", workspacePath, logDebug);
	const globalPolicyRules = buildPolicyRules(globalSettings, "global", globalPath, logDebug);
	const workspacePolicyOverrides = buildPolicyOverrides(
		workspaceSettings,
		"workspace",
		workspacePath,
		logDebug,
	);
	const globalPolicyOverrides = buildPolicyOverrides(globalSettings, "global", globalPath, logDebug);

	return {
		rules: {
			allow: [...workspaceRules.allow, ...globalRules.allow],
			ask: [...workspaceRules.ask, ...globalRules.ask],
			deny: [...workspaceRules.deny, ...globalRules.deny],
		},
		policyRules: [...workspacePolicyRules, ...globalPolicyRules],
		policyOverrides: [...workspacePolicyOverrides, ...globalPolicyOverrides],
	};
}
