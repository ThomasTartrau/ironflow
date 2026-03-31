import { useEffect } from "react";

const SUFFIX = "Ironflow";

interface DocumentMetaOptions {
	title: string;
	description?: string;
}

export function useDocumentMeta({ title, description }: DocumentMetaOptions) {
	useEffect(() => {
		document.title = `${title} - ${SUFFIX}`;

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
	}, [title, description]);
}
