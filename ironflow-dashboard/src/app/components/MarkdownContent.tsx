import Markdown from "react-markdown";

interface MarkdownContentProps {
	content: string;
}

export function MarkdownContent({ content }: MarkdownContentProps) {
	return (
		<div className="markdown-content text-sm leading-relaxed text-foreground break-words [overflow-wrap:anywhere]">
			<Markdown>{content}</Markdown>
		</div>
	);
}
