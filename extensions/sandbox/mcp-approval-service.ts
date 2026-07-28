export const MCP_APPROVAL_SERVICE_KEY = Symbol.for("@behzad/pi-mcp-approval:service:v1");

export interface McpApprovalService {
	version: 1;
	owner: object;
	isEndpointApproved(endpoint: string): boolean;
}

export function publishMcpApprovalService(service: McpApprovalService): void {
	(globalThis as Record<symbol, unknown>)[MCP_APPROVAL_SERVICE_KEY] = service;
}

export function unpublishMcpApprovalService(owner: object): void {
	const registry = globalThis as Record<symbol, unknown>;
	const current = registry[MCP_APPROVAL_SERVICE_KEY] as Partial<McpApprovalService> | undefined;
	if (current?.owner === owner) delete registry[MCP_APPROVAL_SERVICE_KEY];
}
