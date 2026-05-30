import { useEffect, useRef, useState } from "react";
import { codeToHtml } from "shiki";

interface CodeBlockProps {
	code: string;
	language?: string;
}

export function CodeBlock({ code, language = "rust" }: CodeBlockProps) {
	const [html, setHtml] = useState<string | null>(null);
	const cancelledRef = useRef(false);

	useEffect(() => {
		cancelledRef.current = false;

		codeToHtml(code, {
			lang: language,
			theme: "github-dark-dimmed",
		}).then((result) => {
			if (!cancelledRef.current) {
				setHtml(result);
			}
		});

		return () => {
			cancelledRef.current = true;
		};
	}, [code, language]);

	if (!html) {
		return (
			<pre className="animate-pulse opacity-60 rounded-[var(--radius-md)] border bg-muted/30 p-4 text-sm font-mono overflow-auto">
				<code>{code}</code>
			</pre>
		);
	}

	return (
		<div
			className="rounded-[var(--radius-md)] border overflow-auto text-sm [&_pre]:p-4 [&_pre]:m-0 [&_pre]:bg-transparent"
			dangerouslySetInnerHTML={{ __html: html }}
		/>
	);
}
