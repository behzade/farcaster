export function stableStringify(value: unknown): string | undefined {
	try {
		return JSON.stringify(sortKeys(value));
	} catch (error) {
		return undefined;
	}
}

export function sortKeys(value: unknown): unknown {
	if (Array.isArray(value)) {
		return value.map((item) => sortKeys(item));
	}
	if (value && typeof value === "object") {
		const record = value as Record<string, unknown>;
		const keys = Object.keys(record).sort();
		const result: Record<string, unknown> = {};
		for (const key of keys) {
			result[key] = sortKeys(record[key]);
		}
		return result;
	}
	return value;
}

export function deepEqual(a: unknown, b: unknown): boolean {
	if (a === b) return true;
	if (Array.isArray(a) && Array.isArray(b)) {
		if (a.length !== b.length) return false;
		for (let i = 0; i < a.length; i++) {
			if (!deepEqual(a[i], b[i])) return false;
		}
		return true;
	}
	if (
		a &&
		b &&
		typeof a === "object" &&
		typeof b === "object" &&
		!Array.isArray(a) &&
		!Array.isArray(b)
	) {
		const recordA = a as Record<string, unknown>;
		const recordB = b as Record<string, unknown>;
		const keysA = Object.keys(recordA);
		const keysB = Object.keys(recordB);
		if (keysA.length !== keysB.length) return false;
		for (const key of keysA) {
			if (!Object.prototype.hasOwnProperty.call(recordB, key)) return false;
			if (!deepEqual(recordA[key], recordB[key])) return false;
		}
		return true;
	}
	return false;
}
