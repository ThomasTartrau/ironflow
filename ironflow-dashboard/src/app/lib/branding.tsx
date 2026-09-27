import {
	createContext,
	useContext,
	useEffect,
	useState,
	type ReactNode,
} from "react";
import { useTheme } from "@/app/lib/theme";

interface BrandingConfig {
	name: string;
	logoUrl: string;
	/** Logo shown when the dark theme is active. Falls back to `logoUrl`. */
	logoDarkUrl: string;
	faviconUrl: string;
	copyright: string;
	storagePrefix: string;
	description: string;
	theme: Record<string, string>;
	darkTheme: Record<string, string>;
}

const DEFAULT_BRANDING: BrandingConfig = {
	name: "ironflow",
	logoUrl: "/logo.svg",
	logoDarkUrl: "/logo-dark.svg",
	faviconUrl: "/favicon.svg",
	copyright: "ironflow",
	storagePrefix: "ironflow",
	description: "Workflow orchestration dashboard",
	theme: {},
	darkTheme: {},
};

const BrandingContext = createContext<BrandingConfig>(DEFAULT_BRANDING);

export function useBranding(): BrandingConfig {
	return useContext(BrandingContext);
}

/**
 * Logo URL for the theme currently applied to the document.
 */
export function useBrandLogo(): string {
	const { logoUrl, logoDarkUrl } = useBranding();
	const { resolvedTheme } = useTheme();
	return resolvedTheme === "dark" ? logoDarkUrl : logoUrl;
}

/**
 * Merge a `branding.json` payload over the defaults. A custom `logoUrl` without
 * `logoDarkUrl` is used in both themes, so a custom logo never falls back to the
 * default dark one.
 */
export function mergeBranding(data: Partial<BrandingConfig>): BrandingConfig {
	return {
		...DEFAULT_BRANDING,
		...data,
		logoDarkUrl:
			data.logoDarkUrl ?? data.logoUrl ?? DEFAULT_BRANDING.logoDarkUrl,
		theme: { ...DEFAULT_BRANDING.theme, ...data.theme },
		darkTheme: { ...DEFAULT_BRANDING.darkTheme, ...data.darkTheme },
	};
}

function applyThemeOverrides(
	theme: Record<string, string>,
	darkTheme: Record<string, string>,
) {
	const styleId = "branding-theme-overrides";
	let style = document.getElementById(styleId) as HTMLStyleElement | null;
	if (!style) {
		style = document.createElement("style");
		style.id = styleId;
		document.head.appendChild(style);
	}

	const lightVars = Object.entries(theme)
		.map(([key, value]) => `  --${key}: ${value};`)
		.join("\n");
	const darkVars = Object.entries(darkTheme)
		.map(([key, value]) => `  --${key}: ${value};`)
		.join("\n");

	style.textContent = [
		lightVars ? `:root {\n${lightVars}\n}` : "",
		darkVars ? `.dark {\n${darkVars}\n}` : "",
	]
		.filter(Boolean)
		.join("\n");
}

function applyHeadMeta(config: BrandingConfig) {
	document.title = config.name;

	const favicon = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
	if (favicon) {
		favicon.href = config.faviconUrl;
		favicon.type = config.faviconUrl.endsWith(".svg")
			? "image/svg+xml"
			: "image/x-icon";
	}

	const meta = document.querySelector<HTMLMetaElement>(
		'meta[name="description"]',
	);
	if (meta) {
		meta.content = `${config.name} - ${config.description}`;
	}
}

interface BrandingProviderProps {
	children: ReactNode;
}

export function BrandingProvider({ children }: BrandingProviderProps) {
	const [config, setConfig] = useState<BrandingConfig>(DEFAULT_BRANDING);

	useEffect(() => {
		fetch("/branding.json")
			.then((response) => {
				if (!response.ok) return DEFAULT_BRANDING;
				return response.json() as Promise<Partial<BrandingConfig>>;
			})
			.then((data) => {
				const merged = mergeBranding(data);
				setConfig(merged);
				applyThemeOverrides(merged.theme, merged.darkTheme);
				applyHeadMeta(merged);
			})
			.catch(() => {});
	}, []);

	return <BrandingContext value={config}>{children}</BrandingContext>;
}
