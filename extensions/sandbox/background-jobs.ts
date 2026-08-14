import { spawn } from "node:child_process";
import { isAbsolute } from "node:path";

const DEFAULT_SOCKET = "/tmp/pi-agent-tmux.sock";
const MAX_OUTPUT_BYTES = 2 * 1024 * 1024;
const MODEL_MAX_OUTPUT_BYTES = 50 * 1024;
const MODEL_MAX_OUTPUT_LINES = 2000;

export interface BackgroundJobResult {
	exitCode: number;
	output: string;
}

/** Bound a read result before its first model-visible emission; tmux keeps the full history. */
export function modelVisibleBackgroundJobOutput(action: string, output: string): string {
	if (action !== "read") return output;
	const lines = output.split("\n");
	const totalBytes = Buffer.byteLength(output);
	if (lines.length <= MODEL_MAX_OUTPUT_LINES && totalBytes <= MODEL_MAX_OUTPUT_BYTES) return output;

	const notice = [
		`[Background job read truncated from ${lines.length} lines (${totalBytes} bytes) to the model-output limit.`,
		"Full tmux history is preserved. Read fewer lines for a targeted tail, or redirect future job output to a workspace log for complete inspection.]",
	].join("\n");
	const separator = "\n\n";
	const outputByteBudget = MODEL_MAX_OUTPUT_BYTES - Buffer.byteLength(notice + separator);
	const outputLineBudget = MODEL_MAX_OUTPUT_LINES - notice.split("\n").length - 2;
	let kept = lines.slice(-outputLineBudget);
	while (kept.length > 1 && Buffer.byteLength(kept.join("\n")) > outputByteBudget) kept.shift();
	let tail = kept.join("\n");
	if (Buffer.byteLength(tail) > outputByteBudget) {
		const bytes = Buffer.from(tail);
		let start = bytes.length - outputByteBudget;
		while (start < bytes.length && (bytes[start]! & 0xc0) === 0x80) start++;
		tail = bytes.subarray(start).toString("utf8");
	}
	return `${notice}${separator}${tail}`;
}

export function isValidBackgroundJobName(name: string): boolean {
	return /^pi-[A-Za-z0-9._-]{1,60}$/.test(name);
}

export function backgroundJobSocket(
	source: NodeJS.ProcessEnv = process.env,
): string {
	const configured = source.PI_BACKGROUND_TMUX_SOCKET?.trim();
	return configured && isAbsolute(configured) ? configured : DEFAULT_SOCKET;
}

export function isBackgroundJobSocket(
	path: string,
	canonicalizePath: (path: string) => string,
	source: NodeJS.ProcessEnv = process.env,
): boolean {
	return canonicalizePath(path) === canonicalizePath(backgroundJobSocket(source));
}

export function shellJoin(args: readonly string[]): string {
	return args
		.map((value) => `'${value.replaceAll("'", `'\\''`)}'`)
		.join(" ");
}

export function sandboxedJobCommand(
	command: string,
	args: readonly string[],
	environment: Readonly<Record<string, string>>,
): string {
	const cleanEnvironment = Object.entries(environment)
		.sort(([left], [right]) => left.localeCompare(right))
		.map(([name, value]) => `${name}=${value}`);
	return `exec ${shellJoin(["env", "-i", ...cleanEnvironment, command, ...args])}`;
}

export async function runBackgroundJobHelper(
	helperPath: string,
	args: readonly string[],
	options: {
		cwd: string;
		environment: Readonly<Record<string, string>>;
		signal?: AbortSignal;
		timeoutMs?: number;
	},
): Promise<BackgroundJobResult> {
	return await new Promise((resolveResult, reject) => {
		const child = spawn("/bin/bash", [helperPath, ...args], {
			cwd: options.cwd,
			env: {
				...options.environment,
				PI_BACKGROUND_TMUX_SOCKET: backgroundJobSocket(process.env),
			},
			stdio: ["ignore", "pipe", "pipe"],
		});
		const chunks: Buffer[] = [];
		let byteCount = 0;
		let timedOut = false;
		const collect = (data: Buffer) => {
			byteCount += data.length;
			chunks.push(data);
			while (byteCount > MAX_OUTPUT_BYTES && chunks.length > 1) {
				byteCount -= chunks.shift()?.length ?? 0;
			}
		};
		child.stdout?.on("data", collect);
		child.stderr?.on("data", collect);

		const kill = () => child.kill("SIGKILL");
		options.signal?.addEventListener("abort", kill, { once: true });
		const timeout = setTimeout(() => {
			timedOut = true;
			kill();
		}, options.timeoutMs ?? 15_000);

		child.once("error", (error) => {
			clearTimeout(timeout);
			options.signal?.removeEventListener("abort", kill);
			reject(error);
		});
		child.once("close", (code) => {
			clearTimeout(timeout);
			options.signal?.removeEventListener("abort", kill);
			const output = Buffer.concat(chunks).toString("utf8").trim();
			if (options.signal?.aborted) {
				resolveResult({ exitCode: 1, output: output || "Background job request was stopped" });
			} else if (timedOut) {
				resolveResult({ exitCode: 1, output: output || "Background job helper timed out" });
			} else {
				resolveResult({ exitCode: code ?? 1, output });
			}
		});
	});
}
