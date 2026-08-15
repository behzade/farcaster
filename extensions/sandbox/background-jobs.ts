const MODEL_MAX_OUTPUT_BYTES = 50 * 1024;
const MODEL_MAX_OUTPUT_LINES = 2000;

/** Bound a read result before its first model-visible emission. */
export function modelVisibleBackgroundJobOutput(action: string, output: string): string {
	if (action !== "read") return output;
	const lines = output.split("\n");
	const totalBytes = Buffer.byteLength(output);
	if (lines.length <= MODEL_MAX_OUTPUT_LINES && totalBytes <= MODEL_MAX_OUTPUT_BYTES) return output;

	const notice = [
		`[Background job read truncated from ${lines.length} lines (${totalBytes} bytes) to the model-output limit.`,
		"Read fewer lines for a targeted tail, or redirect job output to a workspace log for complete inspection.]",
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
