import type { StatsResponse } from "@/app/lib/types";
import { StatCard } from "@/app/components/StatCard";
import { formatCost, formatPercent } from "@/app/lib/format";
import { Activity, CheckCircle, DollarSign, Zap } from "lucide-react";

interface StatsCardsProps {
	stats: StatsResponse;
}

export function StatsCards({ stats }: StatsCardsProps) {
	return (
		<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
			<StatCard
				label="Total Runs"
				value={stats.total_runs}
				icon={Activity}
				iconClassName="bg-blue-100 text-blue-600 dark:bg-blue-950 dark:text-blue-400"
			/>
			<StatCard
				label="Success Rate"
				value={formatPercent(stats.success_rate_percent)}
				icon={CheckCircle}
				iconClassName="bg-emerald-100 text-emerald-600 dark:bg-emerald-950 dark:text-emerald-400"
			/>
			<StatCard
				label="Active"
				value={stats.active_runs}
				icon={Zap}
				iconClassName="bg-amber-100 text-amber-600 dark:bg-amber-950 dark:text-amber-400"
			/>
			<StatCard
				label="Total Cost"
				value={formatCost(stats.total_cost_usd)}
				icon={DollarSign}
				iconClassName="bg-violet-100 text-violet-600 dark:bg-violet-950 dark:text-violet-400"
			/>
		</div>
	);
}
