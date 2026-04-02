import { useMemo } from "react";
import {
	ReactFlow,
	type Node,
	type Edge,
	Controls,
	Background,
	BackgroundVariant,
	useNodesState,
	useEdgesState,
	type NodeTypes,
	Handle,
	Position,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { StepResponse, StepKind, StepStatus } from "@/app/lib/types";
import { Badge } from "@/components/ui/badge";
import { formatDuration } from "@/app/lib/format";
import { Terminal, Globe, Bot, GitBranch } from "lucide-react";

function getKindMeta(kind: StepKind) {
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

const statusDotColors: Record<StepStatus, string> = {
	completed: "bg-emerald-500",
	failed: "bg-red-500",
	running: "bg-blue-500 animate-pulse",
	pending: "bg-amber-500",
	skipped: "bg-zinc-400",
};

const kindBorderColors: Record<string, string> = {
	amber: "border-amber-400/50",
	blue: "border-blue-400/50",
	purple: "border-purple-400/50",
	indigo: "border-indigo-400/50",
	emerald: "border-emerald-400/50",
};

const kindBgColors: Record<string, string> = {
	amber: "bg-amber-400/10",
	blue: "bg-blue-400/10",
	purple: "bg-purple-400/10",
	indigo: "bg-indigo-400/10",
	emerald: "bg-emerald-400/10",
};

interface StepNodeData {
	step: StepResponse;
	[key: string]: unknown;
}

function StepNode({ data }: { data: StepNodeData }) {
	const step = data.step;
	const meta = getKindMeta(step.kind);
	const Icon = meta.icon;
	const dotColor = statusDotColors[step.status] ?? "bg-zinc-400";
	const borderColor = kindBorderColors[meta.color] ?? "border-zinc-400/50";
	const bgColor = kindBgColors[meta.color] ?? "bg-zinc-400/10";

	return (
		<div
			className={`rounded-lg border-2 ${borderColor} ${bgColor} px-3 py-2 min-w-[140px]`}
		>
			<Handle type="target" position={Position.Left} className="!bg-zinc-400" />
			<div className="flex items-center gap-2">
				<div className={`h-2.5 w-2.5 rounded-full ${dotColor}`} />
				<Icon className="h-3.5 w-3.5 opacity-60" />
				<span className="text-xs font-semibold truncate">{step.name}</span>
			</div>
			{step.duration_ms > 0 && (
				<div className="mt-1 text-[10px] text-foreground/40">
					{formatDuration(step.duration_ms)}
				</div>
			)}
			<Handle
				type="source"
				position={Position.Right}
				className="!bg-zinc-400"
			/>
		</div>
	);
}

const nodeTypes: NodeTypes = {
	step: StepNode,
};

const NODE_WIDTH = 160;
const NODE_HEIGHT = 64;
const H_GAP = 60;
const V_GAP = 20;

interface DagFlowProps {
	steps: StepResponse[];
}

export function DagFlow({ steps }: DagFlowProps) {
	const { nodes, edges } = useMemo(() => {
		// Group steps by position (wave).
		const waveMap = new Map<number, StepResponse[]>();
		for (const step of steps) {
			const list = waveMap.get(step.position) ?? [];
			list.push(step);
			waveMap.set(step.position, list);
		}

		const sortedWaves = Array.from(waveMap.entries()).sort(
			(a, b) => a[0] - b[0],
		);

		const builtNodes: Node[] = [];
		let x = 0;

		for (const [, waveSteps] of sortedWaves) {
			const totalHeight =
				waveSteps.length * NODE_HEIGHT +
				(waveSteps.length - 1) * V_GAP;
			let y = -totalHeight / 2;

			for (const step of waveSteps) {
				builtNodes.push({
					id: step.id,
					type: "step",
					data: { step },
					position: { x, y },
					width: NODE_WIDTH,
					height: NODE_HEIGHT,
				});
				y += NODE_HEIGHT + V_GAP;
			}
			x += NODE_WIDTH + H_GAP;
		}

		// Build edges from dependencies.
		const builtEdges: Edge[] = [];
		for (const step of steps) {
			for (const depId of step.dependencies) {
				builtEdges.push({
					id: `${depId}->${step.id}`,
					source: depId,
					target: step.id,
					animated: step.status === "running",
					style: {
						stroke: step.status === "failed" ? "#ef4444" : "#71717a",
						strokeWidth: 1.5,
					},
				});
			}
		}

		return { nodes: builtNodes, edges: builtEdges };
	}, [steps]);

	const [nodesState, , onNodesChange] = useNodesState(nodes);
	const [edgesState, , onEdgesChange] = useEdgesState(edges);

	const height = Math.max(300, steps.length * 50 + 100);

	return (
		<div className="rounded-2xl border border-border/40 bg-background/60 backdrop-blur overflow-hidden">
			<div className="flex items-center gap-3 px-4 pt-3 pb-2">
				<Badge
					variant="outline"
					className="rounded-full border-indigo-400/40 bg-indigo-400/10 px-2.5 py-0.5 text-[9px] font-semibold uppercase tracking-[0.25em] text-indigo-400"
				>
					DAG
				</Badge>
				<span className="text-[10px] uppercase tracking-[0.25em] text-foreground/40">
					{steps.length} steps
				</span>
			</div>
			<div style={{ height }}>
				<ReactFlow
					nodes={nodesState}
					edges={edgesState}
					onNodesChange={onNodesChange}
					onEdgesChange={onEdgesChange}
					nodeTypes={nodeTypes}
					fitView
					proOptions={{ hideAttribution: true }}
					nodesDraggable={false}
					nodesConnectable={false}
					zoomOnScroll={false}
					panOnScroll
					minZoom={0.5}
					maxZoom={1.5}
				>
					<Background variant={BackgroundVariant.Dots} gap={16} size={1} />
					<Controls showInteractive={false} />
				</ReactFlow>
			</div>
		</div>
	);
}
