export function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function capitalizeFirst(value: string): string {
	return value.length > 0 ? value[0].toUpperCase() + value.slice(1) : value;
}
