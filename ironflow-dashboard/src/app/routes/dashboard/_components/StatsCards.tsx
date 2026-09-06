import type { StatsResponse } from "@/app/lib/types";
import { StatCard } from "@/app/components/StatCard";
import { formatCost, formatPercent } from "@/app/lib/format";
import { Activity, CheckCircle, DollarSign, Zap } from "lucide-react";

interface StatsCardsProps {
	stats: StatsResponse;
}

export function StatsCards({ stats }: StatsCardsProps) {
	const successRate =
		stats.total_runs === 0 ? "--" : formatPercent(stats.success_rate_percent);

	return (
		<div className="space-y-2">
			<div className="flex justify-end">
				<span className="text-xs text-muted-foreground">All time</span>
			</div>
			<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
				<StatCard
					label="Total Runs"
					value={stats.total_runs}
					icon={Activity}
					iconClassName="bg-chart-1/15 text-chart-1"
				/>
				<StatCard
					label="Success Rate"
					value={successRate}
					icon={CheckCircle}
					iconClassName="bg-chart-2/15 text-chart-2"
				/>
				<StatCard
					label="Active"
					value={stats.active_runs}
					icon={Zap}
					iconClassName="bg-chart-3/15 text-chart-3"
				/>
				<StatCard
					label="Total Cost"
					value={formatCost(stats.total_cost_usd)}
					icon={DollarSign}
					iconClassName="bg-chart-4/15 text-chart-4"
				/>
			</div>
		</div>
	);
}
