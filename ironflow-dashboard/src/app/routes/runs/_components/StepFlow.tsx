import { useState, useEffect, useRef, useCallback } from "react";
import type { StepResponse, RunDetailResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { Card } from "@/components/ui/card";
import { formatDuration } from "@/app/lib/format";
import { Link } from "react-router";
import {
	Terminal,
	Globe,
	Bot,
	GitBranch,
	ShieldCheck,
	ZoomIn,
	ZoomOut,
	type LucideIcon,
} from "lucide-react";

type FlowItem =
	| { kind: "step"; step: StepResponse; ownerRunId: string }
	| {
			kind: "group";
			name: string;
			runId: string;
			color: string;
			items: FlowItem[];
	  }
	| {
			kind: "parallel";
			items: FlowItem[];
	  };

function getKindMeta(kind: string): {
	icon: LucideIcon;
	color: string;
	label: string;
} {
	switch (kind) {
		case "shell":
			return { icon: Terminal, color: "amber", label: "shell" };
		case "http":
			return { icon: Globe, color: "blue", label: "http" };
		case "agent":
			return { icon: Bot, color: "purple", label: "agent" };
		case "workflow":
			return { icon: GitBranch, color: "indigo", label: "workflow" };
		case "approval":
			return { icon: ShieldCheck, color: "rose", label: "approval" };
		default:
			return { icon: Terminal, color: "emerald", label: kind };
	}
}

const nodeColors: Record<string, string> = {
	amber: "border-amber-400/40 bg-amber-400/10 text-amber-500",
	blue: "border-blue-400/40 bg-blue-400/10 text-blue-500",
	purple: "border-purple-400/40 bg-purple-400/10 text-purple-500",
	indigo: "border-indigo-400/40 bg-indigo-400/10 text-indigo-500",
	emerald: "border-emerald-400/40 bg-emerald-400/10 text-emerald-500",
	rose: "border-rose-400/40 bg-rose-400/10 text-rose-500",
};

const groupColors = [
	{ border: "border-blue-300/50", bg: "bg-blue-50/40", text: "text-blue-400" },
	{
		border: "border-violet-300/50",
		bg: "bg-violet-50/40",
		text: "text-violet-400",
	},
	{
		border: "border-amber-300/50",
		bg: "bg-amber-50/40",
		text: "text-amber-500",
	},
	{
		border: "border-emerald-300/50",
		bg: "bg-emerald-50/40",
		text: "text-emerald-400",
	},
];

const statusDot: Record<string, string> = {
	completed: "bg-emerald-500",
	failed: "bg-red-500",
	running: "bg-blue-500 animate-pulse",
	pending: "bg-amber-500",
	cancelled: "bg-gray-400",
};

function formatNodeCost(usd: number): string {
	if (usd === 0) return "";
	return `$${usd.toFixed(2)}`;
}

function StepNode({
	step,
	ownerRunId,
	currentRunId,
}: {
	step: StepResponse;
	ownerRunId: string;
	currentRunId: string;
}) {
	const meta = getKindMeta(step.kind);
	const Icon = meta.icon;
	const cost = formatNodeCost(step.cost_usd);
	const dot = statusDot[step.status] ?? "bg-gray-300";
	const isLocal = ownerRunId === currentRunId;

	return (
		<button
			type="button"
			className="block shrink-0 w-[120px] transition-transform hover:scale-105 text-left"
			onClick={() => {
				window.__focusStepTarget = step.id;
				window.dispatchEvent(
					new CustomEvent("focus-step", { detail: step.id }),
				);
				if (!isLocal) {
					window.dispatchEvent(
						new CustomEvent("focus-step-nested", {
							detail: { stepId: step.id },
						}),
					);
				}
				const tryScroll = (attempts: number) => {
					const el = document.getElementById(`step-${step.id}`);
					if (el) {
						el.scrollIntoView({ behavior: "smooth", block: "center" });
						setTimeout(() => {
							window.__focusStepTarget = null;
						}, 2000);
					} else if (attempts > 0) {
						setTimeout(() => tryScroll(attempts - 1), 200);
					}
				};
				tryScroll(10);
			}}
		>
			<Card
				className={`relative w-full overflow-hidden rounded-lg border ${nodeColors[meta.color]} bg-background/70 px-2 py-1.5 backdrop-blur cursor-pointer`}
			>
				<div className="space-y-0.5">
					<div className="flex items-center gap-1">
						<div
							className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${nodeColors[meta.color]} bg-background/80`}
						>
							<Icon className="h-2.5 w-2.5" />
						</div>
						<span className="text-[8px] uppercase tracking-wider text-foreground/50 font-medium">
							{meta.label}
						</span>
						<div
							className={`ml-auto h-1.5 w-1.5 shrink-0 rounded-full ${dot}`}
						/>
					</div>
					<h3 className="truncate text-[11px] font-semibold text-foreground leading-tight">
						{step.name}
					</h3>
					<div className="flex items-center gap-1 text-[9px] text-foreground/45">
						<span>{formatDuration(step.duration_ms)}</span>
						{cost && (
							<>
								<span className="text-foreground/20">·</span>
								<span>{cost}</span>
							</>
						)}
					</div>
				</div>
			</Card>
		</button>
	);
}

function DashedArrow() {
	return (
		<svg
			width="28"
			height="8"
			viewBox="0 0 28 8"
			className="shrink-0 self-center text-foreground/20"
			aria-hidden="true"
		>
			<line
				x1="0"
				y1="4"
				x2="20"
				y2="4"
				stroke="currentColor"
				strokeWidth="1.5"
				strokeDasharray="4,3"
				strokeLinecap="round"
			/>
			<polygon points="20,1 28,4 20,7" fill="currentColor" />
		</svg>
	);
}

function FlowItemView({
	item,
	currentRunId,
}: {
	item: FlowItem;
	currentRunId: string;
}) {
	if (item.kind === "step") {
		return (
			<StepNode
				step={item.step}
				ownerRunId={item.ownerRunId}
				currentRunId={currentRunId}
			/>
		);
	}

	if (item.kind === "parallel") {
		return (
			<div className="flex flex-col gap-1.5 shrink-0 rounded-lg border border-dashed border-foreground/15 bg-foreground/[0.02] px-1.5 py-1.5">
				<span className="text-[8px] uppercase tracking-[0.2em] text-foreground/30 font-semibold px-0.5">
					parallel
				</span>
				{item.items.map((child) => (
					<FlowItemView
						key={getItemKey(child)}
						item={child}
						currentRunId={currentRunId}
					/>
				))}
			</div>
		);
	}

	const depth = getGroupDepth(item);
	const gc = groupColors[depth % groupColors.length];

	return (
		<div
			className={`relative rounded-xl border ${gc.border} ${gc.bg} px-2.5 pb-2.5 pt-5 shrink-0`}
		>
			<Link
				to={`/runs/${item.runId}`}
				className={`absolute top-1 left-2.5 text-[9px] font-semibold uppercase tracking-[0.15em] ${gc.text} hover:underline`}
				onClick={(e) => e.stopPropagation()}
			>
				{item.name}
			</Link>
			<div className="flex items-center gap-1">
				{item.items.map((child, i) => (
					<div key={getItemKey(child)} className="flex items-center">
						{i > 0 && <DashedArrow />}
						<FlowItemView item={child} currentRunId={currentRunId} />
					</div>
				))}
			</div>
		</div>
	);
}

function getGroupDepth(item: FlowItem): number {
	if (item.kind === "step") return 0;
	let max = 0;
	for (const child of item.items) {
		if (child.kind === "group" || child.kind === "parallel") {
			max = Math.max(max, 1 + getGroupDepth(child));
		}
	}
	return max;
}

function getItemKey(item: FlowItem): string {
	if (item.kind === "step") return item.step.id;
	if (item.kind === "parallel")
		return `parallel-${item.items.map(getItemKey).join(",")}`;
	return `group-${item.name}`;
}

async function resolveStep(
	step: StepResponse,
	ownerRunId: string,
): Promise<FlowItem> {
	if (
		step.kind === "workflow" &&
		step.output &&
		typeof step.output.run_id === "string"
	) {
		const childRunId = step.output.run_id;
		const res = await api
			.get<RunDetailResponse>(`/runs/${childRunId}`)
			.catch(() => null);

		if (res && res.data.steps.length > 0) {
			const childItems = await resolveItems(res.data.steps, childRunId);
			return {
				kind: "group",
				name: step.name,
				runId: childRunId,
				color: "blue",
				items: childItems,
			};
		}
	}
	return { kind: "step", step, ownerRunId };
}

async function resolveItems(
	steps: StepResponse[],
	ownerRunId: string,
): Promise<FlowItem[]> {
	// Group steps by position to detect parallel waves.
	const waves = new Map<number, StepResponse[]>();
	for (const step of steps) {
		const list = waves.get(step.position) ?? [];
		list.push(step);
		waves.set(step.position, list);
	}

	const sortedWaves = Array.from(waves.entries()).sort((a, b) => a[0] - b[0]);

	const items: FlowItem[] = [];
	for (const [, waveSteps] of sortedWaves) {
		if (waveSteps.length === 1) {
			items.push(await resolveStep(waveSteps[0], ownerRunId));
		} else {
			const parallelItems = await Promise.all(
				waveSteps.map((s) => resolveStep(s, ownerRunId)),
			);
			items.push({ kind: "parallel", items: parallelItems });
		}
	}

	return items;
}

interface StepFlowProps {
	steps: StepResponse[];
	workflowName: string;
	runId: string;
}

const MIN_ZOOM = 0.3;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.15;

export function StepFlow({ steps, workflowName, runId }: StepFlowProps) {
	const [items, setItems] = useState<FlowItem[] | null>(null);
	const [zoom, setZoom] = useState(1);
	const [pan, setPan] = useState({ x: 0, y: 0 });
	const [isDragging, setIsDragging] = useState(false);
	const dragStart = useRef({ x: 0, y: 0, panX: 0, panY: 0 });
	const containerRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (steps.length === 0) return;
		resolveItems(steps, runId)
			.then(setItems)
			.catch(() => setItems(null));
	}, [steps, runId]);

	const zoomIn = useCallback(
		() => setZoom((z) => Math.min(MAX_ZOOM, z + ZOOM_STEP)),
		[],
	);
	const zoomOut = useCallback(
		() => setZoom((z) => Math.max(MIN_ZOOM, z - ZOOM_STEP)),
		[],
	);
	const zoomReset = useCallback(() => {
		setZoom(1);
		setPan({ x: 0, y: 0 });
	}, []);

	const handleWheel = useCallback((e: React.WheelEvent) => {
		if (e.ctrlKey || e.metaKey) {
			e.preventDefault();
			setZoom((z) => {
				const delta = e.deltaY > 0 ? -ZOOM_STEP : ZOOM_STEP;
				return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z + delta));
			});
		}
	}, []);

	const handleMouseDown = useCallback(
		(e: React.MouseEvent) => {
			if (e.button !== 0) return;
			setIsDragging(true);
			dragStart.current = {
				x: e.clientX,
				y: e.clientY,
				panX: pan.x,
				panY: pan.y,
			};
		},
		[pan],
	);

	const handleMouseMove = useCallback(
		(e: React.MouseEvent) => {
			if (!isDragging) return;
			setPan({
				x: dragStart.current.panX + (e.clientX - dragStart.current.x),
				y: dragStart.current.panY + (e.clientY - dragStart.current.y),
			});
		},
		[isDragging],
	);

	const handleMouseUp = useCallback(() => {
		setIsDragging(false);
	}, []);

	if (!items || items.length === 0) return null;

	const rootGroup: FlowItem = {
		kind: "group",
		name: workflowName,
		runId,
		color: "emerald",
		items,
	};

	return (
		<div className="rounded-2xl border border-border/40 bg-background/60 backdrop-blur p-4">
			<div className="mb-3 flex items-center justify-end gap-1">
				<button
					type="button"
					onClick={zoomOut}
					className="rounded p-1 text-foreground/40 hover:text-foreground/70 hover:bg-foreground/5 transition-colors"
					title="Zoom out"
				>
					<ZoomOut className="h-3.5 w-3.5" />
				</button>
				<button
					type="button"
					onClick={zoomReset}
					className="rounded px-1.5 py-0.5 text-[9px] text-foreground/40 hover:text-foreground/70 hover:bg-foreground/5 transition-colors tabular-nums"
					title="Reset zoom"
				>
					{Math.round(zoom * 100)}%
				</button>
				<button
					type="button"
					onClick={zoomIn}
					className="rounded p-1 text-foreground/40 hover:text-foreground/70 hover:bg-foreground/5 transition-colors"
					title="Zoom in"
				>
					<ZoomIn className="h-3.5 w-3.5" />
				</button>
			</div>
			{/* biome-ignore lint/a11y/noStaticElementInteractions: canvas drag/pan requires mouse events on container */}
			<div
				ref={containerRef}
				className="overflow-hidden pb-1"
				style={{ cursor: isDragging ? "grabbing" : "grab" }}
				onWheel={handleWheel}
				onMouseDown={handleMouseDown}
				onMouseMove={handleMouseMove}
				onMouseUp={handleMouseUp}
				onMouseLeave={handleMouseUp}
			>
				<div
					style={{
						transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
						transformOrigin: "top left",
					}}
				>
					<FlowItemView item={rootGroup} currentRunId={runId} />
				</div>
			</div>
		</div>
	);
}
