import { formatDuration } from "@/app/lib/format";

export interface TimeMarker {
	ms: number;
	label: string;
}

const INTERVALS = [
	50, 100, 250, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000,
	300000, 600000, 1800000, 3600000,
];

const TARGET_COUNT = 6;

export function computeTimeMarkers(totalMs: number): TimeMarker[] {
	if (totalMs <= 0) return [];

	const rawInterval = totalMs / TARGET_COUNT;
	const interval =
		INTERVALS.find((i) => i >= rawInterval) ?? INTERVALS[INTERVALS.length - 1];

	const markers: TimeMarker[] = [];
	let current = 0;
	while (current <= totalMs) {
		markers.push({ ms: current, label: formatDuration(current) });
		current += interval;
	}
	const last = markers[markers.length - 1];
	if (last.ms < totalMs) {
		if ((totalMs - last.ms) / totalMs < 0.15) {
			markers.pop();
		}
		markers.push({ ms: totalMs, label: formatDuration(totalMs) });
	}
	return markers;
}
