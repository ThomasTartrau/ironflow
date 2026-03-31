export function capitalize(str: string): string {
	return str.charAt(0).toUpperCase() + str.slice(1);
}

export function formatDuration(ms: number): string {
	if (ms < 1000) return `${ms}ms`;
	if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
	const minutes = Math.floor(ms / 60000);
	const seconds = Math.floor((ms % 60000) / 1000);
	return `${minutes}m ${seconds}s`;
}

export function formatPercent(value: number, decimals = 1): string {
	return `${value.toFixed(decimals)}%`;
}

export function formatCost(usd: number): string {
	if (usd === 0) return "$0";
	return `$${usd.toFixed(2)}`;
}
