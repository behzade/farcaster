import { Type } from "typebox";

const AccessRightParams = Type.Union([
	Type.Object(
		{
			kind: Type.Literal("filesystem"),
			access: Type.Union([Type.Literal("read"), Type.Literal("write")]),
			path: Type.String({
				description: "Project-relative or home-relative (~/); absolute denial paths under those roots are converted before storage",
				maxLength: 1024,
			}),
			scope: Type.Union([Type.Literal("file"), Type.Literal("tree")]),
		},
		{ additionalProperties: false },
	),
	Type.Object(
		{
			kind: Type.Literal("network_host"),
			host: Type.String({ description: "One exact hostname or IP, without scheme, port, path, or wildcard" }),
		},
		{ additionalProperties: false },
	),
	Type.Object({ kind: Type.Literal("network_local") }, { additionalProperties: false }),
	Type.Object(
		{
			kind: Type.Literal("development_cache"),
			environment: Type.Record(
				Type.String({ pattern: "^[A-Za-z_][A-Za-z0-9_]*$", maxLength: 64 }),
				Type.String({ maxLength: 256 }),
				{ minProperties: 1, maxProperties: 16 },
			),
		},
		{ additionalProperties: false },
	),
]);

export const RequestAccessParams = Type.Object(
	{
		rights: Type.Array(AccessRightParams, { minItems: 1, maxItems: 32 }),
		reason: Type.String({ description: "Why the project needs these rights", maxLength: 2000 }),
	},
	{ additionalProperties: false },
);

export const BashParams = Type.Object(
	{
		command: Type.String({ description: "Bash command to execute" }),
		timeout: Type.Optional(Type.Number({ description: "Timeout in seconds (optional, no default timeout)" })),
	},
	{ additionalProperties: false },
);

export const BackgroundJobParams = Type.Union([
	Type.Object({
		action: Type.Literal("start"),
		name: Type.String({ description: "Unique job name starting with pi-" }),
		command: Type.String({ description: "Shell command to run in the background" }),
		cwd: Type.Optional(Type.String({ description: "Working directory inside this workspace" })),
	}, { additionalProperties: false }),
	Type.Object({ action: Type.Literal("list") }, { additionalProperties: false }),
	Type.Object({ action: Type.Literal("status"), name: Type.String() }, { additionalProperties: false }),
	Type.Object({
		action: Type.Literal("read"),
		name: Type.String(),
		lines: Type.Optional(Type.Integer({ minimum: 1, maximum: 10_000 })),
	}, { additionalProperties: false }),
	Type.Object({ action: Type.Literal("write"), name: Type.String(), text: Type.String() }, { additionalProperties: false }),
	Type.Object({ action: Type.Literal("line"), name: Type.String(), text: Type.String() }, { additionalProperties: false }),
	Type.Object({
		action: Type.Literal("keys"),
		name: Type.String(),
		keys: Type.Array(Type.String(), { minItems: 1, maxItems: 20 }),
	}, { additionalProperties: false }),
	Type.Object({ action: Type.Literal("stop"), name: Type.String() }, { additionalProperties: false }),
], { type: "object" });
