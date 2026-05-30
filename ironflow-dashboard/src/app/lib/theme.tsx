import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
	useSyncExternalStore,
	type ReactNode,
} from "react";

/**
 * Theme mode chosen by the user. "system" follows the OS preference.
 */
export type ThemeMode = "light" | "dark" | "system";

/**
 * The concrete theme actually applied to the document once "system" is resolved.
 */
export type ResolvedTheme = "light" | "dark";

/**
 * localStorage key under which the chosen mode is persisted.
 */
const STORAGE_KEY = "ironflow-ui-theme";

const SYSTEM_DARK_QUERY = "(prefers-color-scheme: dark)";

interface ThemeContextValue {
	/** The user's chosen mode. */
	theme: ThemeMode;
	/** The theme currently applied to <html> ("system" resolved against the OS). */
	resolvedTheme: ResolvedTheme;
	/** Change the mode; the document class and persistence update as a side effect. */
	setTheme: (mode: ThemeMode) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function readStoredTheme(): ThemeMode {
	try {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored === "light" || stored === "dark" || stored === "system") {
			return stored;
		}
	} catch {
		// localStorage unavailable (private mode / quota); fall through to default.
	}
	return "dark";
}

function subscribeSystemDark(onChange: () => void): () => void {
	const media = window.matchMedia(SYSTEM_DARK_QUERY);
	media.addEventListener("change", onChange);
	return () => media.removeEventListener("change", onChange);
}

function getSystemDarkSnapshot(): boolean {
	return window.matchMedia(SYSTEM_DARK_QUERY).matches;
}

interface ThemeProviderProps {
	children: ReactNode;
}

/**
 * Owns the dashboard theme. Reads the initial mode from localStorage (default "dark"),
 * subscribes to the OS preference for "system", applies the "dark" class to <html>, and
 * persists the chosen mode. The class is applied once this provider mounts.
 */
export function ThemeProvider({ children }: ThemeProviderProps) {
	const [theme, setTheme] = useState<ThemeMode>(readStoredTheme);

	// External subscription (not derived state): the OS color-scheme preference.
	const systemDark = useSyncExternalStore(
		subscribeSystemDark,
		getSystemDarkSnapshot,
		() => false,
	);

	// Derived at render, never via useEffect.
	const resolvedTheme: ResolvedTheme =
		theme === "system" ? (systemDark ? "dark" : "light") : theme;

	// Side effect only: reflect the resolved theme onto the document element.
	useEffect(() => {
		document.documentElement.classList.toggle("dark", resolvedTheme === "dark");
	}, [resolvedTheme]);

	// Side effect only: persist the chosen mode.
	useEffect(() => {
		try {
			localStorage.setItem(STORAGE_KEY, theme);
		} catch {
			// localStorage unavailable; persistence is best-effort.
		}
	}, [theme]);

	const setThemeMode = useCallback((mode: ThemeMode) => setTheme(mode), []);

	const value = useMemo<ThemeContextValue>(
		() => ({ theme, resolvedTheme, setTheme: setThemeMode }),
		[theme, resolvedTheme, setThemeMode],
	);

	return <ThemeContext value={value}>{children}</ThemeContext>;
}

/**
 * Access the current theme and a setter. Must be used within a [`ThemeProvider`].
 */
export function useTheme(): ThemeContextValue {
	const context = useContext(ThemeContext);
	if (context === null) {
		throw new Error("useTheme must be used within a ThemeProvider");
	}
	return context;
}
