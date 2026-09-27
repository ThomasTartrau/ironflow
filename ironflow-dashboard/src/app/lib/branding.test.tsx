import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { mergeBranding, useBrandLogo } from "./branding";
import { ThemeProvider, useTheme } from "./theme";

const STORAGE_KEY = "ironflow-ui-theme";

function Logo() {
	const logoUrl = useBrandLogo();
	const { setTheme } = useTheme();
	return (
		<div>
			<span data-testid="logo">{logoUrl}</span>
			<button type="button" onClick={() => setTheme("light")}>
				light
			</button>
			<button type="button" onClick={() => setTheme("dark")}>
				dark
			</button>
		</div>
	);
}

function renderLogo() {
	return render(
		<ThemeProvider>
			<Logo />
		</ThemeProvider>,
	);
}

describe("mergeBranding", () => {
	it("keeps the default light and dark logos when nothing is overridden", () => {
		const merged = mergeBranding({});
		expect(merged.logoUrl).toBe("/logo.svg");
		expect(merged.logoDarkUrl).toBe("/logo-dark.svg");
	});

	it("uses a custom logo in both themes when no dark logo is given", () => {
		const merged = mergeBranding({ logoUrl: "/acme.png" });
		expect(merged.logoUrl).toBe("/acme.png");
		expect(merged.logoDarkUrl).toBe("/acme.png");
	});

	it("keeps a custom dark logo next to a custom light logo", () => {
		const merged = mergeBranding({
			logoUrl: "/acme.png",
			logoDarkUrl: "/acme-dark.png",
		});
		expect(merged.logoUrl).toBe("/acme.png");
		expect(merged.logoDarkUrl).toBe("/acme-dark.png");
	});

	it("merges theme overrides over the defaults", () => {
		const merged = mergeBranding({ theme: { primary: "#000" } });
		expect(merged.theme).toEqual({ primary: "#000" });
		expect(merged.darkTheme).toEqual({});
		expect(merged.name).toBe("ironflow");
	});
});

describe("useBrandLogo", () => {
	beforeEach(() => {
		localStorage.clear();
		document.documentElement.classList.remove("dark");
	});

	it("returns the dark logo under the default dark theme", () => {
		renderLogo();
		expect(screen.getByTestId("logo").textContent).toBe("/logo-dark.svg");
	});

	it("returns the light logo when the light theme is stored", () => {
		localStorage.setItem(STORAGE_KEY, "light");
		renderLogo();
		expect(screen.getByTestId("logo").textContent).toBe("/logo.svg");
	});

	it("follows a theme change", () => {
		renderLogo();
		fireEvent.click(screen.getByText("light"));
		expect(screen.getByTestId("logo").textContent).toBe("/logo.svg");
		fireEvent.click(screen.getByText("dark"));
		expect(screen.getByTestId("logo").textContent).toBe("/logo-dark.svg");
	});
});
