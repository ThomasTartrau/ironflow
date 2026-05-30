import Markdown from "react-markdown";
import type { Components } from "react-markdown";

interface MarkdownContentProps {
	content: string;
}

const markdownComponents: Components = {
	a: ({ href, children, ...props }) => (
		<a
			href={href}
			className="text-primary underline underline-offset-2 hover:text-primary/80"
			{...props}
		>
			{children}
		</a>
	),
};

export function MarkdownContent({ content }: MarkdownContentProps) {
	return (
		<div className="markdown-content text-sm leading-relaxed text-foreground break-words [overflow-wrap:anywhere]">
			<Markdown components={markdownComponents}>{content}</Markdown>
		</div>
	);
}
