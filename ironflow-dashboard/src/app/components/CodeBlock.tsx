import { useEffect, useState } from "react";
import { codeToHtml } from "shiki";

interface CodeBlockProps {
	code: string;
	language?: string;
}

export function CodeBlock({ code, language = "rust" }: CodeBlockProps) {
	const [html, setHtml] = useState<string | null>(null);

	useEffect(() => {
		codeToHtml(code, {
			lang: language,
			theme: "github-light",
		}).then(setHtml);
	}, [code, language]);

	if (!html) {
		return (
			<pre className="rounded-lg border bg-muted/30 p-4 text-sm font-mono overflow-auto">
				<code>{code}</code>
			</pre>
		);
	}

	return (
		<div
			className="rounded-lg border overflow-auto text-sm [&_pre]:p-4 [&_pre]:m-0 [&_pre]:bg-transparent"
			dangerouslySetInnerHTML={{ __html: html }}
		/>
	);
}
