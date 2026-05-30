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
		color: "text-amber-600 dark:text-amber-400",
		barBg: "bg-amber-400/70 dark:bg-amber-400/50",
		barBorder: "border-amber-500/30 dark:border-amber-400/30",
		badgeCls:
			"bg-amber-100 text-amber-700 border-amber-200 dark:bg-amber-400/15 dark:text-amber-300 dark:border-amber-400/30",
		label: "shell",
	},
	agent: {
		icon: Bot,
		color: "text-purple-600 dark:text-purple-400",
		barBg: "bg-purple-400/70 dark:bg-purple-400/50",
		barBorder: "border-purple-500/30 dark:border-purple-400/30",
		badgeCls:
			"bg-purple-100 text-purple-700 border-purple-200 dark:bg-purple-400/15 dark:text-purple-300 dark:border-purple-400/30",
		label: "agent",
	},
	http: {
		icon: Globe,
		color: "text-blue-600 dark:text-blue-400",
		barBg: "bg-blue-400/70 dark:bg-blue-400/50",
		barBorder: "border-blue-500/30 dark:border-blue-400/30",
		badgeCls:
			"bg-blue-100 text-blue-700 border-blue-200 dark:bg-blue-400/15 dark:text-blue-300 dark:border-blue-400/30",
		label: "http",
	},
	workflow: {
		icon: GitBranch,
		color: "text-emerald-600 dark:text-emerald-400",
		barBg: "bg-emerald-400/70 dark:bg-emerald-400/50",
		barBorder: "border-emerald-500/30 dark:border-emerald-400/30",
		badgeCls:
			"bg-emerald-100 text-emerald-700 border-emerald-200 dark:bg-emerald-400/15 dark:text-emerald-300 dark:border-emerald-400/30",
		label: "workflow",
	},
	approval: {
		icon: ShieldCheck,
		color: "text-rose-600 dark:text-rose-400",
		barBg: "bg-rose-400/70 dark:bg-rose-400/50",
		barBorder: "border-rose-500/30 dark:border-rose-400/30",
		badgeCls:
			"bg-rose-100 text-rose-700 border-rose-200 dark:bg-rose-400/15 dark:text-rose-300 dark:border-rose-400/30",
		label: "approval",
	},
};

const defaultKindMeta: KindMeta = {
	icon: Terminal,
	color: "text-muted-foreground",
	barBg: "bg-muted-foreground/50",
	barBorder: "border-border",
	badgeCls: "bg-muted text-muted-foreground border-border",
	label: "unknown",
};

export function getKindMeta(kind: StepKind): KindMeta {
	return kindMetaMap[kind] ?? { ...defaultKindMeta, label: kind };
}

export const statusBarModifier: Partial<Record<StepStatus, string>> = {
	running: "animate-pulse ring-1 ring-primary/60",
	failed: "!bg-destructive/70 !border-destructive/40",
	pending: "!bg-muted-foreground/30 !border-border",
	skipped: "!bg-muted-foreground/15 !border-border/50",
};
