import type { ExtensionContext } from "@earendil-works/pi-coding-agent";

export function notify(ctx: ExtensionContext, message: string): void {
	if (ctx.hasUI) {
		ctx.ui.notify(message, "info");
	}
}
