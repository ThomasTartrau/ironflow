import { useState, useCallback } from "react";

export function usePersistedOpen(
	key: string,
	defaultOpen: boolean,
): [boolean, () => void] {
	const storageKey = `ironflow:${key}`;
	const [open, setOpen] = useState(() => {
		const stored = localStorage.getItem(storageKey);
		if (stored !== null) return stored === "true";
		return defaultOpen;
	});

	const toggle = useCallback(() => {
		setOpen((prev) => {
			const next = !prev;
			localStorage.setItem(storageKey, String(next));
			return next;
		});
	}, [storageKey]);

	return [open, toggle];
}
