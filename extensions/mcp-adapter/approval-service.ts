export const MCP_APPROVAL_SERVICE_KEY = Symbol.for("@behzad/pi-mcp-approval:service:v1");

export interface McpApprovalService {
	version: 1;
	owner: object;
	isEndpointApproved(endpoint: string): boolean;
}

export function currentMcpApprovalService(): McpApprovalService | undefined {
	const value = (globalThis as Record<symbol, unknown>)[MCP_APPROVAL_SERVICE_KEY];
	if (!value || typeof value !== "object") return undefined;
	const service = value as Partial<McpApprovalService>;
	return service.version === 1 && typeof service.isEndpointApproved === "function"
		? (service as McpApprovalService)
		: undefined;
}

export function requireMcpEndpointApproval(endpoint: string): void {
	const service = currentMcpApprovalService();
	if (!service) {
		throw new Error("MCP access is blocked because the sandbox approval service is unavailable");
	}
	if (!service.isEndpointApproved(endpoint)) {
		throw new Error(`MCP endpoint is not approved for this session or workspace: ${endpoint}`);
	}
}
