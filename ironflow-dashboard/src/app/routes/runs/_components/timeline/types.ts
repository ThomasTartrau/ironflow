import type { StepResponse, StepKind, StepStatus } from "@/app/lib/types";
import {
	Terminal,
	Globe,
	Bot,
	GitBranch,
	ShieldCheck,
	type LucideIcon,
} from "lucide-react";

export interface FlatStep {
	step: StepResponse;
	ownerRunId: string;
	startMs: number;
	endMs: number;
	depth: number;
}

export interface TimelineRow {
	step: StepResponse;
	ownerRunId: string;
	startMs: number;
	endMs: number;
	depth: number;
	offsetPercent: number;
	widthPercent: number;
}

export const ROW_HEIGHT = 28;
export const ROW_GAP = 2;
export const AXIS_HEIGHT = 22;
export const MIN_BAR_PX = 4;

export interface KindMeta {
	icon: LucideIcon;
	color: string;
	barBg: string;
	barBorder: string;
	badgeCls: string;
	label: string;
}

const kindMetaMap: Record<string, KindMeta> = {
	shell: {
		icon: Terminal,
		color: "text-amber-600",
		barBg: "bg-amber-400/80",
		barBorder: "border-amber-500/30",
		badgeCls: "bg-amber-100 text-amber-700 border-amber-200",
		label: "shell",
	},
	agent: {
		icon: Bot,
		color: "text-purple-600",
		barBg: "bg-purple-400/80",
		barBorder: "border-purple-500/30",
		badgeCls: "bg-purple-100 text-purple-700 border-purple-200",
		label: "agent",
	},
	http: {
		icon: Globe,
		color: "text-blue-600",
		barBg: "bg-blue-400/80",
		barBorder: "border-blue-500/30",
		badgeCls: "bg-blue-100 text-blue-700 border-blue-200",
		label: "http",
	},
	workflow: {
		icon: GitBranch,
		color: "text-emerald-600",
		barBg: "bg-emerald-400/80",
		barBorder: "border-emerald-500/30",
		badgeCls: "bg-emerald-100 text-emerald-700 border-emerald-200",
		label: "workflow",
	},
	approval: {
		icon: ShieldCheck,
		color: "text-rose-600",
		barBg: "bg-rose-400/80",
		barBorder: "border-rose-500/30",
		badgeCls: "bg-rose-100 text-rose-700 border-rose-200",
		label: "approval",
	},
};

const defaultKindMeta: KindMeta = {
	icon: Terminal,
	color: "text-gray-600",
	barBg: "bg-gray-400/80",
	barBorder: "border-gray-500/30",
	badgeCls: "bg-gray-100 text-gray-700 border-gray-200",
	label: "unknown",
};

export function getKindMeta(kind: StepKind): KindMeta {
	return kindMetaMap[kind] ?? { ...defaultKindMeta, label: kind };
}

export const statusBarModifier: Partial<Record<StepStatus, string>> = {
	running: "animate-pulse",
	failed: "!bg-red-400/80 !border-red-500/30",
	pending: "!bg-gray-300/60 !border-gray-400/30",
	skipped: "!bg-gray-200/40 !border-gray-300/20",
};
