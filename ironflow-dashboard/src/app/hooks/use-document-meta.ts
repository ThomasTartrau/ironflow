import { useEffect } from "react";
import { useBranding } from "@/app/lib/branding";

interface DocumentMetaOptions {
	title: string;
	description?: string;
}

export function useDocumentMeta({ title, description }: DocumentMetaOptions) {
	const branding = useBranding();

	useEffect(() => {
		document.title = `${title} - ${branding.name}`;

		if (description) {
			let meta = document.querySelector<HTMLMetaElement>(
				'meta[name="description"]',
			);
			if (!meta) {
				meta = document.createElement("meta");
				meta.name = "description";
				document.head.appendChild(meta);
			}
			meta.content = description;
		}
	}, [title, description, branding.name]);
}
