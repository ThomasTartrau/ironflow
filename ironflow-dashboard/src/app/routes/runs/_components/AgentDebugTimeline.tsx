import { useState } from "react";
import {
	Brain,
	ChevronDown,
	ChevronRight,
	Cpu,
	MessageSquare,
	Wrench,
	AlertTriangle,
	CheckCircle2,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { MarkdownContent } from "@/app/components/MarkdownContent";

interface DebugToolCall {
	id?: string | null;
	name: string;
	input: unknown;
}

interface DebugToolResult {
	tool_use_id?: string | null;
	content: unknown;
	is_error?: boolean;
}

interface DebugMessage {
	text?: string | null;
	thinking?: string | null;
	thinking_redacted?: boolean;
	tool_calls: DebugToolCall[];
	tool_results?: DebugToolResult[];
	stop_reason?: string | null;
	input_tokens?: number | null;
	output_tokens?: number | null;
}

function isDebugMessageArray(value: unknown): value is DebugMessage[] {
	return (
		Array.isArray(value) &&
		value.every(
			(m) =>
				typeof m === "object" &&
				m !== null &&
				"tool_calls" in m &&
				Array.isArray((m as { tool_calls: unknown }).tool_calls),
		)
	);
}

function formatToolInput(input: unknown): string {
	if (input === null || input === undefined) return "";
	if (typeof input === "string") return input;
	return JSON.stringify(input, null, 2);
}

function formatToolResult(content: unknown): string {
	if (content === null || content === undefined) return "";
	if (typeof content === "string") return content;
	if (Array.isArray(content)) {
		return content
			.map((block) => {
				if (typeof block === "string") return block;
				if (
					typeof block === "object" &&
					block !== null &&
					"text" in block &&
					typeof (block as { text: unknown }).text === "string"
				) {
					return (block as { text: string }).text;
				}
				return JSON.stringify(block);
			})
			.join("\n");
	}
	return JSON.stringify(content, null, 2);
}

function truncate(value: string, max: number): string {
	if (value.length <= max) return value;
	return `${value.slice(0, max)}...`;
}

function getToolColor(name: string): string {
	const lower = name.toLowerCase();
	if (lower.includes("bash") || lower.includes("shell")) {
		return "bg-amber-100 text-amber-700 border-amber-200 dark:bg-amber-400/15 dark:text-amber-300 dark:border-amber-400/30";
	}
	if (
		lower.includes("read") ||
		lower.includes("grep") ||
		lower.includes("glob")
	) {
		return "bg-sky-100 text-sky-700 border-sky-200 dark:bg-sky-400/15 dark:text-sky-300 dark:border-sky-400/30";
	}
	if (lower.includes("edit") || lower.includes("write")) {
		return "bg-emerald-100 text-emerald-700 border-emerald-200 dark:bg-emerald-400/15 dark:text-emerald-300 dark:border-emerald-400/30";
	}
	if (lower.includes("web") || lower.includes("fetch")) {
		return "bg-violet-100 text-violet-700 border-violet-200 dark:bg-violet-400/15 dark:text-violet-300 dark:border-violet-400/30";
	}
	return "bg-muted text-muted-foreground border-border";
}

interface ThinkingBlockProps {
	text: string;
}

function ThinkingBlock({ text }: ThinkingBlockProps) {
	const [open, setOpen] = useState(false);
	const preview = truncate(text.replace(/\n+/g, " "), 120);
	const contentId = `thinking-content-${text.slice(0, 8).replace(/\s/g, "")}`;

	return (
		<div className="rounded-[var(--radius-sm)] border border-dashed border-indigo-200 bg-indigo-50/40 dark:border-indigo-400/25 dark:bg-indigo-400/10">
			<button
				type="button"
				onClick={() => setOpen(!open)}
				aria-expanded={open}
				aria-controls={contentId}
				className="w-full flex items-start gap-2 px-3 py-2 text-left hover:bg-indigo-50/60 dark:hover:bg-indigo-400/10 transition-colors cursor-pointer"
			>
				{open ? (
					<ChevronDown className="w-3 h-3 text-indigo-500 dark:text-indigo-400 shrink-0 mt-1" />
				) : (
					<ChevronRight className="w-3 h-3 text-indigo-500 dark:text-indigo-400 shrink-0 mt-1" />
				)}
				<Brain
					className="w-3.5 h-3.5 text-indigo-500 dark:text-indigo-400 shrink-0 mt-0.5"
					aria-hidden="true"
				/>
				<span className="text-xs font-semibold text-indigo-700 dark:text-indigo-300 shrink-0">
					Thinking
				</span>
				{!open && (
					<span className="text-xs text-indigo-600/80 dark:text-indigo-400/80 italic truncate">
						{preview}
					</span>
				)}
			</button>
			{open && (
				<div id={contentId} className="px-3 pb-3 pl-8">
					<pre className="text-xs whitespace-pre-wrap break-words text-indigo-900/90 dark:text-indigo-200/90 font-mono leading-relaxed">
						{text}
					</pre>
				</div>
			)}
		</div>
	);
}

function RedactedThinkingBlock() {
	return (
		<div className="rounded-[var(--radius-sm)] border border-dashed border-indigo-200 bg-indigo-50/40 dark:border-indigo-400/25 dark:bg-indigo-400/10 px-3 py-2 flex items-center gap-2">
			<Brain
				className="w-3.5 h-3.5 text-indigo-500 dark:text-indigo-400 shrink-0"
				aria-hidden="true"
			/>
			<span className="text-xs font-semibold text-indigo-700 dark:text-indigo-300">
				Thinking
			</span>
			<span className="text-xs text-indigo-600/70 dark:text-indigo-400/70 italic">
				redacted by the model (signature only)
			</span>
		</div>
	);
}

interface ToolCallBlockProps {
	call: DebugToolCall;
	result?: DebugToolResult;
}

function ToolCallBlock({ call, result }: ToolCallBlockProps) {
	const [open, setOpen] = useState(false);
	const inputText = formatToolInput(call.input);
	const inputPreview = truncate(inputText.replace(/\s+/g, " "), 80);
	const hasError = result?.is_error === true;
	const resultText = result ? formatToolResult(result.content) : null;
	const contentId = `tool-content-${call.id ?? call.name}`;

	return (
		<div
			className={`rounded-[var(--radius-sm)] border ${hasError ? "border-destructive/30 bg-destructive/10" : "border-border bg-muted/20"}`}
		>
			<button
				type="button"
				onClick={() => setOpen(!open)}
				aria-expanded={open}
				aria-controls={contentId}
				className="w-full flex items-start gap-2 px-3 py-2 text-left hover:bg-muted/30 transition-colors cursor-pointer"
			>
				{open ? (
					<ChevronDown className="w-3 h-3 text-muted-foreground shrink-0 mt-1" />
				) : (
					<ChevronRight className="w-3 h-3 text-muted-foreground shrink-0 mt-1" />
				)}
				<Wrench className="w-3.5 h-3.5 text-muted-foreground shrink-0 mt-0.5" />
				<Badge
					variant="outline"
					className={`text-[10px] font-medium shrink-0 ${getToolColor(call.name)}`}
				>
					{call.name}
				</Badge>
				<span className="text-xs text-muted-foreground font-mono truncate">
					{inputPreview}
				</span>
				{result &&
					(hasError ? (
						<AlertTriangle
							className="w-3.5 h-3.5 text-destructive shrink-0 ml-auto"
							aria-hidden="true"
						/>
					) : (
						<CheckCircle2
							className="w-3.5 h-3.5 text-[var(--status-completed-fg)] shrink-0 ml-auto"
							aria-hidden="true"
						/>
					))}
			</button>
			{open && (
				<div id={contentId} className="px-3 pb-3 pl-8 space-y-2">
					<div>
						<div className="text-[10px] uppercase tracking-wide font-semibold text-muted-foreground mb-1">
							Input
						</div>
						<pre className="text-xs max-h-48 overflow-auto whitespace-pre-wrap break-words bg-background border border-border rounded-[var(--radius-sm)] p-2 font-mono leading-relaxed">
							{inputText || "(no input)"}
						</pre>
					</div>
					{resultText !== null && (
						<div>
							<div
								className={`text-[10px] uppercase tracking-wide font-semibold mb-1 ${hasError ? "text-destructive" : "text-[var(--status-completed-fg)]"}`}
							>
								{hasError ? "Error" : "Result"}
							</div>
							<pre
								className={`text-xs max-h-64 overflow-auto whitespace-pre-wrap break-words border rounded-[var(--radius-sm)] p-2 font-mono leading-relaxed ${hasError ? "bg-destructive/10 text-destructive border-destructive/20" : "bg-background border-border"}`}
							>
								{resultText || "(no content)"}
							</pre>
						</div>
					)}
				</div>
			)}
		</div>
	);
}

interface AssistantTextBlockProps {
	text: string;
}

function AssistantTextBlock({ text }: AssistantTextBlockProps) {
	return (
		<div className="rounded-[var(--radius-sm)] border border-violet-200 bg-violet-50/30 dark:border-violet-400/25 dark:bg-violet-400/10 px-3 py-2">
			<div className="flex items-center gap-2 mb-1">
				<MessageSquare
					className="w-3.5 h-3.5 text-violet-500 dark:text-violet-400"
					aria-hidden="true"
				/>
				<span className="text-xs font-semibold text-violet-700 dark:text-violet-300">
					Assistant
				</span>
			</div>
			<div className="pl-5">
				<MarkdownContent content={text} />
			</div>
		</div>
	);
}

interface TurnProps {
	index: number;
	message: DebugMessage;
}

function Turn({ index, message }: TurnProps) {
	const results = message.tool_results ?? [];
	const resultsByCall = new Map<string, DebugToolResult>();
	for (const result of results) {
		if (result.tool_use_id) {
			resultsByCall.set(result.tool_use_id, result);
		}
	}

	const hasTokens =
		typeof message.input_tokens === "number" ||
		typeof message.output_tokens === "number";

	return (
		<div className="space-y-2">
			<div className="flex items-center gap-2">
				<div className="w-6 h-6 rounded-full bg-violet-100 text-violet-700 text-xs font-semibold flex items-center justify-center shrink-0 dark:bg-violet-400/15 dark:text-violet-300 tabular-nums">
					{index + 1}
				</div>
				<span className="text-xs font-semibold text-foreground">
					Turn {index + 1}
				</span>
				{message.stop_reason && (
					<Badge variant="outline" className="text-[10px] font-medium">
						{message.stop_reason}
					</Badge>
				)}
				{hasTokens && (
					<span className="flex items-center gap-1 text-[11px] text-muted-foreground ml-auto">
						<Cpu className="w-3 h-3" />
						{(message.input_tokens ?? 0).toLocaleString()} in /{" "}
						{(message.output_tokens ?? 0).toLocaleString()} out
					</span>
				)}
			</div>
			<div className="pl-8 space-y-2">
				{message.thinking && <ThinkingBlock text={message.thinking} />}
				{!message.thinking && message.thinking_redacted && (
					<RedactedThinkingBlock />
				)}
				{message.text && <AssistantTextBlock text={message.text} />}
				{message.tool_calls.map((call, i) => {
					const result = call.id ? resultsByCall.get(call.id) : undefined;
					return (
						<ToolCallBlock
							key={call.id ?? `${call.name}-${i}`}
							call={call}
							result={result}
						/>
					);
				})}
			</div>
		</div>
	);
}

interface AgentDebugTimelineProps {
	debugMessages: unknown;
}

export function countVisibleTurns(debugMessages: unknown): number {
	if (!isDebugMessageArray(debugMessages)) return 0;
	return debugMessages.filter((m) => !isTurnEmpty(m)).length;
}

function isTurnEmpty(message: DebugMessage): boolean {
	const hasThinking =
		typeof message.thinking === "string" && message.thinking.length > 0;
	const hasRedacted = message.thinking_redacted === true;
	const hasText = typeof message.text === "string" && message.text.length > 0;
	const hasCalls = message.tool_calls.length > 0;
	const hasResults = (message.tool_results ?? []).length > 0;
	return !hasThinking && !hasRedacted && !hasText && !hasCalls && !hasResults;
}

export function AgentDebugTimeline({ debugMessages }: AgentDebugTimelineProps) {
	if (!isDebugMessageArray(debugMessages) || debugMessages.length === 0) {
		return null;
	}

	const visibleMessages = debugMessages.filter((m) => !isTurnEmpty(m));
	if (visibleMessages.length === 0) {
		return null;
	}

	return (
		<div className="space-y-4 border-l-2 border-border pl-4">
			{visibleMessages.map((message, i) => (
				<Turn key={`turn-${i}`} index={i} message={message} />
			))}
		</div>
	);
}
