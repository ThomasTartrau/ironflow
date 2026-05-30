import { useEffect, useRef, useState, useCallback } from "react";
import type { LogEntry } from "@/app/hooks/use-log-stream";
import { useLogStream } from "@/app/hooks/use-log-stream";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ArrowDown, Trash2, Pause, Play } from "lucide-react";

interface LogStreamPanelProps {
	runId: string;
	enabled: boolean;
}

const STREAM_STYLES: Record<string, { text: string; badge: string }> = {
	stderr: {
		text: "text-destructive",
		badge: "bg-destructive/10 text-destructive border-destructive/30",
	},
	system: {
		text: "text-[var(--status-retrying-fg)]",
		badge:
			"bg-[var(--status-retrying-bg)] text-[var(--status-retrying-fg)] border-[var(--status-retrying-border)]",
	},
	stdout: {
		text: "text-[var(--status-completed-fg)]",
		badge:
			"bg-[var(--status-completed-bg)] text-[var(--status-completed-fg)] border-[var(--status-completed-border)]",
	},
};

const DEFAULT_STREAM_STYLE = STREAM_STYLES.stdout;

function LogLine({ entry }: { entry: LogEntry }) {
	const style = STREAM_STYLES[entry.stream] ?? DEFAULT_STREAM_STYLE;
	return (
		<div className="flex gap-2 py-0.5 hover:bg-muted/20 group">
			<Badge
				variant="outline"
				className={`text-[9px] font-mono shrink-0 h-4 px-1 ${style.badge}`}
			>
				{entry.stream}
			</Badge>
			<span className="text-[10px] text-muted-foreground/60 shrink-0 font-mono tabular-nums">
				{entry.stepName}
			</span>
			<span
				className={`text-xs font-mono whitespace-pre-wrap break-all ${style.text}`}
			>
				{entry.line}
			</span>
		</div>
	);
}

export function LogStreamPanel({ runId, enabled }: LogStreamPanelProps) {
	const [paused, setPaused] = useState(false);
	const [autoScroll, setAutoScroll] = useState(true);
	const containerRef = useRef<HTMLDivElement>(null);
	const frozenLinesRef = useRef<LogEntry[]>([]);
	const { lines, clear } = useLogStream({ runId, enabled });

	const scrollToBottom = useCallback(() => {
		const el = containerRef.current;
		if (el) {
			el.scrollTop = el.scrollHeight;
		}
	}, []);

	const togglePause = useCallback(() => {
		setPaused((prev) => {
			if (!prev) {
				frozenLinesRef.current = lines;
			}
			return !prev;
		});
	}, [lines]);

	useEffect(() => {
		if (lines.length > 0 && autoScroll && !paused) {
			scrollToBottom();
		}
	}, [lines, autoScroll, paused, scrollToBottom]);

	const handleScroll = useCallback(() => {
		const el = containerRef.current;
		if (!el) return;
		const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
		setAutoScroll(atBottom);
	}, []);

	const visibleLines = paused ? frozenLinesRef.current : lines;

	return (
		<div className="rounded-[var(--radius)] border border-border bg-card overflow-hidden">
			<div className="flex items-center justify-between px-3 py-1.5 border-b border-border bg-muted/30">
				<div className="flex items-center gap-2">
					<div
						className={`w-2 h-2 rounded-full ${enabled && !paused ? "bg-[var(--status-running-fg)] animate-pulse" : "bg-muted-foreground/40"}`}
					/>
					<span className="text-xs font-medium text-muted-foreground">
						Live Logs
					</span>
					<Badge
						variant="outline"
						className="text-[10px] text-muted-foreground border-border"
					>
						{lines.length} lines
					</Badge>
				</div>
				<div className="flex items-center gap-1">
					{!autoScroll && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
							onClick={() => {
								setAutoScroll(true);
								scrollToBottom();
							}}
						>
							<ArrowDown className="w-3 h-3 mr-1" />
							Scroll to bottom
						</Button>
					)}
					<Button
						variant="ghost"
						size="sm"
						className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
						onClick={togglePause}
						title={paused ? "Resume" : "Pause"}
					>
						{paused ? (
							<Play className="w-3 h-3" />
						) : (
							<Pause className="w-3 h-3" />
						)}
					</Button>
					<Button
						variant="ghost"
						size="sm"
						className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
						onClick={clear}
						title="Clear logs"
					>
						<Trash2 className="w-3 h-3" />
					</Button>
				</div>
			</div>
			<div
				ref={containerRef}
				onScroll={handleScroll}
				className="h-80 overflow-y-auto px-3 py-2 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent"
			>
				{visibleLines.length === 0 ? (
					<div className="flex items-center justify-center h-full text-xs text-muted-foreground/60">
						{enabled ? "Waiting for log output..." : "Run is not active"}
					</div>
				) : (
					visibleLines.map((entry, i) => (
						<LogLine key={`${entry.stepId}-${entry.at}-${i}`} entry={entry} />
					))
				)}
			</div>
		</div>
	);
}
