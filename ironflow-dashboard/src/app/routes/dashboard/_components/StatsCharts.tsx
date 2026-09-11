import { useEffect, useState, useCallback } from "react";
import {
	BarChart,
	Bar,
	LineChart,
	Line,
	AreaChart,
	Area,
	XAxis,
	YAxis,
	CartesianGrid,
	Tooltip,
	Legend,
	ResponsiveContainer,
} from "recharts";
import type { StatsHistoryBucketResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { formatDuration, formatCost } from "@/app/lib/format";

export const PERIODS = ["24h", "7d", "30d", "90d"] as const;
export type Period = (typeof PERIODS)[number];

interface StatsChartsProps {
	workflowFilter: string;
	period: Period;
}

function formatTime(time: string, period: Period): string {
	const d = new Date(time);
	if (period === "24h") {
		return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}
	return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

function chartColor(index: number): string {
	const colors = [
		"var(--chart-1)",
		"var(--chart-2)",
		"var(--chart-3)",
		"var(--chart-4)",
		"var(--chart-5)",
	];
	return colors[index % colors.length];
}

export function StatsCharts({ workflowFilter, period }: StatsChartsProps) {
	const [buckets, setBuckets] = useState<StatsHistoryBucketResponse[]>([]);
	const [loading, setLoading] = useState(true);

	const fetchHistory = useCallback(async () => {
		setLoading(true);
		const params = new URLSearchParams({ period });
		if (workflowFilter) params.set("workflow", workflowFilter);
		const res = await api.get<{
			period: string;
			granularity: string;
			workflow: string | null;
			buckets: StatsHistoryBucketResponse[];
		}>(`/stats/history?${params}`);
		setBuckets(res.data.buckets);
		setLoading(false);
	}, [period, workflowFilter]);

	useEffect(() => {
		fetchHistory();
	}, [fetchHistory]);

	const chartData = buckets.map((b) => ({
		time: formatTime(b.time, period),
		completed: b.completed,
		failed: b.failed,
		cancelled: b.cancelled,
		avg_duration_ms: b.avg_duration_ms,
		p95_duration_ms: b.p95_duration_ms,
		total_cost_usd: Number(b.total_cost_usd),
		success_rate:
			b.completed + b.failed > 0
				? Math.round((b.completed / (b.completed + b.failed)) * 100)
				: 0,
	}));

	return (
		<div
			aria-busy={loading}
			className={
				loading ? "opacity-50 pointer-events-none transition-opacity" : ""
			}
		>
			{chartData.length === 0 && !loading ? (
				<p className="text-sm text-muted-foreground text-center py-8">
					No data for this period.
				</p>
			) : (
				<div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
					<ChartCard title="Volume & Status">
						<ResponsiveContainer width="100%" height={220}>
							<BarChart data={chartData}>
								<CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
								<XAxis
									dataKey="time"
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
								/>
								<YAxis
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
									allowDecimals={false}
								/>
								<Tooltip
									contentStyle={{
										background: "var(--popover)",
										border: "1px solid var(--border)",
										borderRadius: 6,
										fontSize: 12,
									}}
								/>
								<Legend wrapperStyle={{ fontSize: 12 }} />
								<Bar
									dataKey="completed"
									stackId="status"
									fill={chartColor(1)}
									name="Completed"
								/>
								<Bar
									dataKey="failed"
									stackId="status"
									fill="var(--destructive)"
									name="Failed"
								/>
								<Bar
									dataKey="cancelled"
									stackId="status"
									fill={chartColor(3)}
									name="Cancelled"
								/>
							</BarChart>
						</ResponsiveContainer>
					</ChartCard>

					<ChartCard title="Duration (avg & p95)">
						<ResponsiveContainer width="100%" height={220}>
							<LineChart data={chartData}>
								<CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
								<XAxis
									dataKey="time"
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
								/>
								<YAxis
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
									tickFormatter={(v: number) => formatDuration(v)}
								/>
								<Tooltip
									contentStyle={{
										background: "var(--popover)",
										border: "1px solid var(--border)",
										borderRadius: 6,
										fontSize: 12,
									}}
									formatter={(v) => formatDuration(Number(v ?? 0))}
								/>
								<Legend wrapperStyle={{ fontSize: 12 }} />
								<Line
									type="monotone"
									dataKey="avg_duration_ms"
									stroke={chartColor(0)}
									name="Avg"
									dot={false}
									strokeWidth={2}
								/>
								<Line
									type="monotone"
									dataKey="p95_duration_ms"
									stroke={chartColor(2)}
									name="P95"
									dot={false}
									strokeWidth={2}
									strokeDasharray="4 2"
								/>
							</LineChart>
						</ResponsiveContainer>
					</ChartCard>

					<ChartCard title="Cumulative Cost">
						<ResponsiveContainer width="100%" height={220}>
							<AreaChart data={chartData}>
								<CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
								<XAxis
									dataKey="time"
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
								/>
								<YAxis
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
									tickFormatter={(v: number) => formatCost(v)}
								/>
								<Tooltip
									contentStyle={{
										background: "var(--popover)",
										border: "1px solid var(--border)",
										borderRadius: 6,
										fontSize: 12,
									}}
									formatter={(v) => formatCost(Number(v ?? 0))}
								/>
								<Area
									type="monotone"
									dataKey="total_cost_usd"
									stroke={chartColor(3)}
									fill={chartColor(3)}
									fillOpacity={0.15}
									name="Cost (USD)"
									strokeWidth={2}
								/>
							</AreaChart>
						</ResponsiveContainer>
					</ChartCard>

					<ChartCard title="Success Rate (%)">
						<ResponsiveContainer width="100%" height={220}>
							<LineChart data={chartData}>
								<CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
								<XAxis
									dataKey="time"
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
								/>
								<YAxis
									tick={{ fontSize: 11 }}
									stroke="var(--muted-foreground)"
									domain={[0, 100]}
									tickFormatter={(v: number) => `${v}%`}
								/>
								<Tooltip
									contentStyle={{
										background: "var(--popover)",
										border: "1px solid var(--border)",
										borderRadius: 6,
										fontSize: 12,
									}}
									formatter={(v) => `${v ?? 0}%`}
								/>
								<Line
									type="monotone"
									dataKey="success_rate"
									stroke={chartColor(1)}
									name="Success Rate"
									dot={false}
									strokeWidth={2}
								/>
							</LineChart>
						</ResponsiveContainer>
					</ChartCard>
				</div>
			)}
		</div>
	);
}

function ChartCard({
	title,
	children,
}: {
	title: string;
	children: React.ReactNode;
}) {
	return (
		<div className="border border-border rounded-lg p-4 bg-card">
			<h3 className="text-xs font-medium text-muted-foreground mb-3">
				{title}
			</h3>
			{children}
		</div>
	);
}
