import { registerHooks } from "node:module";

const stubs = new Map<string, string>([
	[
		"@earendil-works/pi-ai",
		`export const StringEnum = (values) => ({ type: "string", enum: [...values] });`,
	],
	[
		"@earendil-works/pi-coding-agent",
		`export const DEFAULT_MAX_BYTES = 50 * 1024;
export const DEFAULT_MAX_LINES = 2000;
export const formatSize = (bytes) => bytes + "B";
export const truncateHead = (input, options) => {
  const lines = input.split("\\n");
  let content = lines.slice(0, options.maxLines).join("\\n");
  if (Buffer.byteLength(content) > options.maxBytes) content = Buffer.from(content).subarray(0, options.maxBytes).toString();
  return { content, truncated: content !== input, outputLines: content.split("\\n").length, totalLines: lines.length, outputBytes: Buffer.byteLength(content), totalBytes: Buffer.byteLength(input) };
};`,
	],
	[
		"typebox",
		`const pass = (schema = {}) => schema;
export const Type = {
  Object: (properties) => ({ type: "object", properties }),
  String: (options = {}) => ({ type: "string", ...options }),
  Optional: pass,
  Integer: (options = {}) => ({ type: "integer", ...options }),
  Unsafe: pass,
};`,
	],
]);

registerHooks({
	resolve(specifier, context, nextResolve) {
		const source = stubs.get(specifier);
		if (source !== undefined) {
			return {
				url: `data:text/javascript,${encodeURIComponent(source)}`,
				shortCircuit: true,
			};
		}
		return nextResolve(specifier, context);
	},
});
