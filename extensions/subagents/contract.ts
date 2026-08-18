export const THINKING_LEVELS = [
	"off",
	"minimal",
	"low",
	"medium",
	"high",
	"xhigh",
	"max",
] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];
export type SubagentContext = "fork" | "blank";
export type SendMode = "prompt" | "steer";
export type RunStatus = "running" | "idle" | "failed" | "stopped";

export interface StartRequest {
	prompt: string;
	context?: SubagentContext;
	provider?: string;
	model?: string;
	effort?: ThinkingLevel;
	cwd: string;
	parentSessionFile?: string;
	parentProvider?: string;
	parentModel?: string;
	parentEffort?: ThinkingLevel;
}

export interface ChildSession {
	readonly id: string;
	readonly sessionFile: string;
	readonly provider: string;
	readonly model: string;
	readonly effort: ThinkingLevel;
	isStreaming(): boolean;
	run(prompt: string): Promise<string>;
	send(message: string, mode: SendMode): Promise<void>;
	abort(): Promise<void>;
	dispose(): Promise<void> | void;
}

export interface ChildSessionFactory {
	create(request: StartRequest): Promise<ChildSession>;
}

export interface RunSnapshot {
	id: string;
	sessionId: string;
	sessionFile: string;
	status: RunStatus;
	context: SubagentContext;
	provider: string;
	model: string;
	effort: ThinkingLevel;
	output?: string;
	error?: string;
}
