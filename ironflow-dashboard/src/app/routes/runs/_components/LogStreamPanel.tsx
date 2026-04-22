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
		text: "text-red-400",
		badge: "bg-red-900/50 text-red-300 border-red-700",
	},
	system: {
		text: "text-yellow-400",
		badge: "bg-yellow-900/50 text-yellow-300 border-yellow-700",
	},
	stdout: {
		text: "text-green-400",
		badge: "bg-green-900/50 text-green-300 border-green-700",
	},
};

const DEFAULT_STREAM_STYLE = STREAM_STYLES.stdout;

function LogLine({ entry }: { entry: LogEntry }) {
	const style = STREAM_STYLES[entry.stream] ?? DEFAULT_STREAM_STYLE;
	return (
		<div className="flex gap-2 py-0.5 hover:bg-white/5 group">
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
		<div className="rounded-lg border border-zinc-800 bg-zinc-950 overflow-hidden">
			<div className="flex items-center justify-between px-3 py-1.5 border-b border-zinc-800 bg-zinc-900/50">
				<div className="flex items-center gap-2">
					<div
						className={`w-2 h-2 rounded-full ${enabled && !paused ? "bg-green-500 animate-pulse" : "bg-zinc-600"}`}
					/>
					<span className="text-xs font-medium text-zinc-400">Live Logs</span>
					<Badge
						variant="outline"
						className="text-[10px] text-zinc-500 border-zinc-700"
					>
						{lines.length} lines
					</Badge>
				</div>
				<div className="flex items-center gap-1">
					{!autoScroll && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 px-2 text-xs text-zinc-400 hover:text-zinc-200"
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
						className="h-6 w-6 p-0 text-zinc-400 hover:text-zinc-200"
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
						className="h-6 w-6 p-0 text-zinc-400 hover:text-zinc-200"
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
				className="h-80 overflow-y-auto px-3 py-2 scrollbar-thin scrollbar-thumb-zinc-700 scrollbar-track-transparent"
			>
				{visibleLines.length === 0 ? (
					<div className="flex items-center justify-center h-full text-xs text-zinc-600">
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
