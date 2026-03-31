import { useState, useEffect } from "react";
import type { StepResponse, RunDetailResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { formatDuration } from "@/app/lib/format";
import { Link } from "react-router";
import { Terminal, Globe, Bot, GitBranch, type LucideIcon } from "lucide-react";

type FlowItem =
	| { kind: "step"; step: StepResponse; ownerRunId: string }
	| {
			kind: "group";
			name: string;
			runId: string;
			color: string;
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
		if (child.kind === "group") {
			max = Math.max(max, 1 + getGroupDepth(child));
		}
	}
	return max;
}

function getItemKey(item: FlowItem): string {
	return item.kind === "step" ? item.step.id : `group-${item.name}`;
}

async function resolveItems(
	steps: StepResponse[],
	ownerRunId: string,
): Promise<FlowItem[]> {
	const items: FlowItem[] = [];

	for (const step of steps) {
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
				items.push({
					kind: "group",
					name: step.name,
					runId: childRunId,
					color: "blue",
					items: childItems,
				});
			} else {
				items.push({ kind: "step", step, ownerRunId });
			}
		} else {
			items.push({ kind: "step", step, ownerRunId });
		}
	}

	return items;
}

interface StepFlowProps {
	steps: StepResponse[];
	workflowName: string;
	runId: string;
}

export function StepFlow({ steps, workflowName, runId }: StepFlowProps) {
	const [items, setItems] = useState<FlowItem[] | null>(null);

	useEffect(() => {
		if (steps.length === 0) return;
		resolveItems(steps, runId)
			.then(setItems)
			.catch(() => setItems(null));
	}, [steps, runId]);

	if (!items || items.length === 0) return null;

	const totalSteps = countSteps(items);
	const rootGroup: FlowItem = {
		kind: "group",
		name: workflowName,
		runId,
		color: "emerald",
		items,
	};

	return (
		<div className="rounded-2xl border border-border/40 bg-background/60 backdrop-blur p-4">
			<div className="mb-3 flex items-center gap-3">
				<Badge
					variant="outline"
					className="rounded-full border-emerald-400/40 bg-emerald-400/10 px-2.5 py-0.5 text-[9px] font-semibold uppercase tracking-[0.25em] text-emerald-400"
				>
					Pipeline
				</Badge>
				<span className="text-[10px] uppercase tracking-[0.25em] text-foreground/40">
					{totalSteps} steps
				</span>
			</div>
			<div className="overflow-x-auto pb-1">
				<FlowItemView item={rootGroup} currentRunId={runId} />
			</div>
		</div>
	);
}

function countSteps(items: FlowItem[]): number {
	let count = 0;
	for (const item of items) {
		if (item.kind === "step") count++;
		else count += countSteps(item.items);
	}
	return count;
}
