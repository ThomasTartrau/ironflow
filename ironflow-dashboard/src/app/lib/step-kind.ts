/**
 * Visual language for step kinds, shared by the run step flow and the
 * workflow execution plan so both read as the same diagram.
 */
import {
	Terminal,
	Globe,
	Bot,
	GitBranch,
	ShieldCheck,
	SkipForward,
	type LucideIcon,
} from "lucide-react";

export interface KindMeta {
	icon: LucideIcon;
	color: string;
	label: string;
}

export function getKindMeta(kind: string): KindMeta {
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
		case "skip":
			return { icon: SkipForward, color: "slate", label: "skipped" };
		default:
			return { icon: Terminal, color: "emerald", label: kind };
	}
}

// Node color classes are safe for both light and dark: alpha-based palette colors
// remain readable in dark mode because they use opacity fractions, not absolute lightness.
export const nodeColors: Record<string, string> = {
	amber:
		"border-amber-400/40 bg-amber-400/10 text-amber-400 dark:text-amber-300",
	blue: "border-blue-400/40 bg-blue-400/10 text-blue-400 dark:text-blue-300",
	purple:
		"border-purple-400/40 bg-purple-400/10 text-purple-400 dark:text-purple-300",
	indigo:
		"border-indigo-400/40 bg-indigo-400/10 text-indigo-400 dark:text-indigo-300",
	emerald:
		"border-emerald-400/40 bg-emerald-400/10 text-emerald-400 dark:text-emerald-300",
	rose: "border-rose-400/40 bg-rose-400/10 text-rose-400 dark:text-rose-300",
	slate:
		"border-slate-400/40 bg-slate-400/10 text-slate-400 dark:text-slate-300",
};
