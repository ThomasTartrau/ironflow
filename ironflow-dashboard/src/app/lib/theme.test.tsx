import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ThemeProvider, useTheme } from "./theme";

const STORAGE_KEY = "ironflow-ui-theme";

function Consumer() {
	const { theme, resolvedTheme, setTheme } = useTheme();
	return (
		<div>
			<span data-testid="mode">{theme}</span>
			<span data-testid="resolved">{resolvedTheme}</span>
			<button type="button" onClick={() => setTheme("light")}>
				light
			</button>
			<button type="button" onClick={() => setTheme("dark")}>
				dark
			</button>
			<button type="button" onClick={() => setTheme("system")}>
				system
			</button>
		</div>
	);
}

function renderProvider() {
	return render(
		<ThemeProvider>
			<Consumer />
		</ThemeProvider>,
	);
}

function htmlHasDark(): boolean {
	return document.documentElement.classList.contains("dark");
}

describe("ThemeProvider", () => {
	beforeEach(() => {
		localStorage.clear();
		document.documentElement.classList.remove("dark");
	});

	it("defaults to dark when nothing is stored", () => {
		renderProvider();
		expect(screen.getByTestId("mode").textContent).toBe("dark");
		expect(screen.getByTestId("resolved").textContent).toBe("dark");
		expect(htmlHasDark()).toBe(true);
	});

	it("reads the stored mode on mount", () => {
		localStorage.setItem(STORAGE_KEY, "light");
		renderProvider();
		expect(screen.getByTestId("mode").textContent).toBe("light");
		expect(screen.getByTestId("resolved").textContent).toBe("light");
		expect(htmlHasDark()).toBe(false);
	});

	it("setTheme('light') removes the dark class and persists", () => {
		renderProvider();
		fireEvent.click(screen.getByText("light"));
		expect(htmlHasDark()).toBe(false);
		expect(localStorage.getItem(STORAGE_KEY)).toBe("light");
	});

	it("setTheme('dark') adds the dark class and persists", () => {
		localStorage.setItem(STORAGE_KEY, "light");
		renderProvider();
		fireEvent.click(screen.getByText("dark"));
		expect(htmlHasDark()).toBe(true);
		expect(localStorage.getItem(STORAGE_KEY)).toBe("dark");
	});

	it("resolves 'system' against the OS preference (no dark in test env)", () => {
		renderProvider();
		fireEvent.click(screen.getByText("system"));
		expect(screen.getByTestId("mode").textContent).toBe("system");
		expect(screen.getByTestId("resolved").textContent).toBe("light");
		expect(htmlHasDark()).toBe(false);
		expect(localStorage.getItem(STORAGE_KEY)).toBe("system");
	});

	it("throws when useTheme is used outside a ThemeProvider", () => {
		expect(() => render(<Consumer />)).toThrow(/ThemeProvider/);
	});
});
