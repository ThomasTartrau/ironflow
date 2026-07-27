import { DollarSign } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { StatCard } from "@/app/components/StatCard";
import { formatCost } from "@/app/lib/format";
import { cn } from "@/lib/utils";

/** Fraction of the cap above which the run is visually flagged. */
export const COST_WARNING_RATIO = 0.8;

/**
 * Share of the cap consumed, clamped to [0, 1] so the bar never overflows.
 *
 * A cap of zero leaves no room at all: any spend reads as full.
 */
export function costRatio(cost: number, cap: number): number {
	if (cap <= 0) return cost > 0 ? 1 : 0;
	return Math.min(1, Math.max(0, cost / cap));
}

/** Visual severity of the current spend against the cap. */
export type CostLevel = "normal" | "warning" | "exceeded";

export function costLevel(cost: number, cap: number): CostLevel {
	const ratio = costRatio(cost, cap);
	if (ratio >= 1) return "exceeded";
	if (ratio >= COST_WARNING_RATIO) return "warning";
	return "normal";
}

const BAR_COLOR: Record<CostLevel, string> = {
	normal: "bg-primary",
	warning: "bg-amber-500",
	exceeded: "bg-destructive",
};

const TEXT_COLOR: Record<CostLevel, string> = {
	normal: "text-muted-foreground",
	warning: "text-amber-600 dark:text-amber-500",
	exceeded: "text-destructive",
};

interface CostBudgetCardProps {
	cost: number;
	/** Cost cap in USD. `null` or `undefined` means the run has no cap. */
	maxCost?: number | null;
}

/**
 * Cost tile for a run.
 *
 * Without a cap this is the plain cost stat. With a cap it adds the cap, a
 * progress bar, and a colour shift past {@link COST_WARNING_RATIO}.
 */
export function CostBudgetCard({ cost, maxCost }: CostBudgetCardProps) {
	if (maxCost === null || maxCost === undefined) {
		return <StatCard label="Cost" value={formatCost(cost)} icon={DollarSign} />;
	}

	const ratio = costRatio(cost, maxCost);
	const level = costLevel(cost, maxCost);
	const percent = Math.round(ratio * 100);

	return (
		<Card className="py-0 gap-0">
			<CardContent className="p-4">
				<div className="flex items-center gap-3">
					<div
						aria-hidden="true"
						className="flex-shrink-0 rounded-[var(--radius-sm)] p-2 bg-muted text-muted-foreground"
					>
						<DollarSign className="w-4 h-4" />
					</div>
					<div className="min-w-0 flex-1">
						<p className="text-xs text-muted-foreground truncate">Cost</p>
						<p className="text-xl font-semibold tracking-tight truncate tabular-nums">
							{formatCost(cost)}
							<span className="text-sm font-normal text-muted-foreground">
								{" / "}
								{formatCost(maxCost)}
							</span>
						</p>
					</div>
				</div>
				<div
					className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-muted"
					role="progressbar"
					aria-label="Cost against budget"
					aria-valuemin={0}
					aria-valuemax={100}
					aria-valuenow={percent}
				>
					<div
						className={cn(
							"h-full rounded-full transition-all",
							BAR_COLOR[level],
						)}
						style={{ width: `${percent}%` }}
					/>
				</div>
				<p className={cn("mt-1.5 text-xs tabular-nums", TEXT_COLOR[level])}>
					{level === "exceeded"
						? `Budget reached (${percent}%)`
						: `${percent}% of budget`}
				</p>
			</CardContent>
		</Card>
	);
}
