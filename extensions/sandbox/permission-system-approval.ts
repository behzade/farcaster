import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Effect } from "effect";

const PERMISSIONS_SERVICE_KEY = Symbol.for("@gotgenes/pi-permission-system:service");

export interface UserApprovalChoice {
	id: string;
	label: string;
	requestReason?: boolean;
}

export interface UserApprovalRequest {
	requestId: string;
	title: string;
	message: string;
	source: "tool_call";
	surface: string | null;
	value: string | null;
	choices: readonly UserApprovalChoice[];
	reasonTitle?: string;
	reasonPlaceholder?: string;
	signal?: AbortSignal;
}

export interface UserApprovalResult {
	choiceId: string | null;
	reason?: string;
	unavailableReason?: string;
}

interface PermissionSystemService {
	requestUserApproval?: (request: UserApprovalRequest) => Promise<UserApprovalResult>;
}

/**
 * Shows an approval locally or delegates it to pi-permission-system when this
 * session is a headless child. The service owns only prompt transport; the
 * sandbox remains responsible for policy validation, atomic persistence, and activation.
 */
export const requestUserApproval: (
	ctx: Pick<ExtensionContext, "hasUI" | "ui">,
	request: UserApprovalRequest,
) => Effect.Effect<UserApprovalResult> = Effect.fn("Sandbox.requestUserApproval")(
	function* (
		ctx: Pick<ExtensionContext, "hasUI" | "ui">,
		request: UserApprovalRequest,
	) {
		if (ctx.hasUI) return yield* requestLocalApproval(ctx.ui, request);

		const service = getPermissionSystemService();
		if (!service?.requestUserApproval) {
			return {
				choiceId: null,
				unavailableReason:
					"pi-permission-system with user-approval forwarding is required for headless approval",
			} satisfies UserApprovalResult;
		}
		return yield* Effect.tryPromise({
			try: () => service.requestUserApproval!(request),
			catch: (error) => error,
		}).pipe(
			Effect.catch((error) => Effect.succeed({
				choiceId: null,
				unavailableReason: error instanceof Error ? error.message : String(error),
			} satisfies UserApprovalResult)),
		);
	},
);

function getPermissionSystemService(): PermissionSystemService | undefined {
	return (globalThis as Record<symbol, unknown>)[
		PERMISSIONS_SERVICE_KEY
	] as PermissionSystemService | undefined;
}

const requestLocalApproval: (
	ui: Pick<ExtensionContext["ui"], "select" | "input">,
	request: UserApprovalRequest,
) => Effect.Effect<UserApprovalResult> = Effect.fn("Sandbox.requestLocalApproval")(
	function* (
		ui: Pick<ExtensionContext["ui"], "select" | "input">,
		request: UserApprovalRequest,
	) {
		const labels = request.choices.map((choice) => choice.label);
		const selection = yield* Effect.promise(() =>
			ui.select(`${request.title}\n${request.message}`, labels, {
				signal: request.signal,
			}),
		);
		const choice = request.choices.find((candidate) => candidate.label === selection);
		if (!choice) return { choiceId: null } satisfies UserApprovalResult;

		const reason = choice.requestReason
			? yield* Effect.promise(() => ui.input(
					request.reasonTitle ?? "Tell the agent what to do instead",
					request.reasonPlaceholder ?? "Short note",
					{ signal: request.signal },
				))
			: undefined;
		return { choiceId: choice.id, ...(reason ? { reason } : {}) } satisfies UserApprovalResult;
	},
);
