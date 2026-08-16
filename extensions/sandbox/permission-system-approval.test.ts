import assert from "node:assert/strict";
import test from "node:test";
import { Effect } from "effect";
import { requestUserApproval, type UserApprovalRequest } from "./permission-system-approval.ts";

const SERVICE_KEY = Symbol.for("@gotgenes/pi-permission-system:service");
const request: UserApprovalRequest = {
	requestId: "request-1",
	title: "Permission needed",
	message: "Allow the requested file?",
	source: "tool_call",
	surface: "read",
	value: "/tmp/input",
	choices: [
		{ id: "allow", label: "Allow" },
		{ id: "deny", label: "No" },
		{ id: "deny-reason", label: "No, with comment", requestReason: true },
	],
};

function setService(service: unknown): void {
	(globalThis as Record<symbol, unknown>)[SERVICE_KEY] = service;
}

function clearService(): void {
	delete (globalThis as Record<symbol, unknown>)[SERVICE_KEY];
}

test.afterEach(clearService);

test("uses the local UI when one is available", async () => {
	setService({ requestUserApproval: async () => { throw new Error("must not delegate"); } });
	const result = await Effect.runPromise(requestUserApproval(
		{
			hasUI: true,
			ui: {
				select: async () => "Allow",
				input: async () => undefined,
			} as never,
		},
		request,
	));
	assert.deepEqual(result, { choiceId: "allow" });
});

test("collects a denial reason from the local UI", async () => {
	const result = await Effect.runPromise(requestUserApproval(
		{
			hasUI: true,
			ui: {
				select: async () => "No, with comment",
				input: async () => "Use the checked-in fixture",
			} as never,
		},
		request,
	));
	assert.deepEqual(result, {
		choiceId: "deny-reason",
		reason: "Use the checked-in fixture",
	});
});

test("delegates a headless prompt through the published permission service", async () => {
	setService({
		requestUserApproval: async (received: UserApprovalRequest) => {
			assert.equal(received, request);
			return { choiceId: "allow" };
		},
	});
	const result = await Effect.runPromise(requestUserApproval({ hasUI: false, ui: {} as never }, request));
	assert.deepEqual(result, { choiceId: "allow" });
});

test("fails closed when the permission service cannot forward approvals", async () => {
	const result = await Effect.runPromise(requestUserApproval({ hasUI: false, ui: {} as never }, request));
	assert.equal(result.choiceId, null);
	assert.match(result.unavailableReason ?? "", /pi-permission-system/);
});
