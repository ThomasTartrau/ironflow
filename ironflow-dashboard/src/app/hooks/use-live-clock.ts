import { useEffect, useState } from "react";

interface UseLiveClockOptions {
	enabled: boolean;
	intervalMs?: number;
}

/**
 * Returns the current wall-clock time (ms) and re-renders the calling
 * component every `intervalMs` while `enabled` is true. When disabled,
 * returns a frozen timestamp (the moment the hook last ticked) so that
 * derived values remain stable.
 */
export function useLiveClock({
	enabled,
	intervalMs = 500,
}: UseLiveClockOptions): number {
	const [now, setNow] = useState<number>(() => Date.now());

	useEffect(() => {
		if (!enabled) return;
		setNow(Date.now());
		const id = setInterval(() => setNow(Date.now()), intervalMs);
		return () => clearInterval(id);
	}, [enabled, intervalMs]);

	return now;
}
