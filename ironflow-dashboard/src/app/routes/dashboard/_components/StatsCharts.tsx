import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
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
import type {
	StatsHistoryBucketResponse,
	StatsHistoryResponse,
} from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { formatDuration, formatCost } from "@/app/lib/format";
import { toFilterParams, type DashboardFilters } from "../stats-filters";
import { STATUS_SERIES, toChartData, type Period } from "./stats-chart-data";

interface StatsChartsProps {
	filters: DashboardFilters;
	period: Period;
	/**
	 * Changes whenever the page data is revalidated (the loader result), so
	 * the history is refetched on the same SSE events as the stats cards.
	 */
	refreshKey?: unknown;
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

/** Shared look for every chart tooltip: opaque in both light and dark theme. */
const CHART_TOOLTIP_CONTENT_STYLE: CSSProperties = {
	background: "var(--popover)",
	border: "1px solid var(--border)",
	borderRadius: 6,
	fontSize: 12,
};

// Recharts renders the legend wrapper after the tooltip wrapper in the DOM,
// so without an explicit z-index the legend paints over the tooltip's last
// rows on hover. Keep the tooltip strictly above the legend on every chart
// that has one.
const CHART_LEGEND_WRAPPER_STYLE: CSSProperties = { fontSize: 12, zIndex: 10 };
const CHART_TOOLTIP_WRAPPER_STYLE: CSSProperties = { zIndex: 20 };

interface VolumeStatusTooltipEntry {
	dataKey?: string | number;
	name?: string;
	value?: number;
	color?: string;
}

interface VolumeStatusTooltipContentProps {
	active?: boolean;
	label?: string;
	payload?: VolumeStatusTooltipEntry[];
}

/** Only lists statuses with a non-zero count for the hovered bucket; an
 * all-zero bucket shows just the date, keeping the tooltip short enough to
 * never reach the legend. */
function VolumeStatusTooltipContent({
	active,
	label,
	payload,
}: VolumeStatusTooltipContentProps) {
	if (!active) return null;
	const entries = (payload ?? []).filter((entry) => (entry.value ?? 0) > 0);
	return (
		<div style={CHART_TOOLTIP_CONTENT_STYLE} className="px-3 py-2">
			<p className="font-medium">{label}</p>
			{entries.map((entry) => (
				<p key={String(entry.dataKey)} style={{ color: entry.color }}>
					{entry.name}: {entry.value}
				</p>
			))}
		</div>
	);
}

export function StatsCharts({ filters, period, refreshKey }: StatsChartsProps) {
	const [buckets, setBuckets] = useState<StatsHistoryBucketResponse[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);

	const params = toFilterParams(filters);
	params.set("period", period);
	const paramsString = params.toString();

	// biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey is not read, it only signals a loader revalidation that must refetch the history.
	useEffect(() => {
		const controller = new AbortController();
		const { signal } = controller;
		setLoading(true);
		api
			.get<StatsHistoryResponse>(`/stats/history?${paramsString}`, { signal })
			.then((res) => {
				if (signal.aborted) return;
				setBuckets(res.data.buckets);
				setError(null);
			})
			.catch((err: unknown) => {
				// A superseded request is aborted on purpose: not an error.
				if (signal.aborted) return;
				setError(err instanceof Error ? err.message : String(err));
			})
			.finally(() => {
				if (!signal.aborted) setLoading(false);
			});
		return () => controller.abort();
	}, [paramsString, refreshKey]);

	const chartData = toChartData(buckets, period);
	// Buckets are zero-filled by the API, so "no data" means no run at all.
	const hasRuns = chartData.some((d) => d.total > 0);

	return (
		<div
			aria-busy={loading}
			className={
				loading ? "opacity-50 pointer-events-none transition-opacity" : ""
			}
		>
			{error && (
				<p role="alert" className="text-sm text-destructive mb-3">
					Could not load trends: {error}
				</p>
			)}
			{!hasRuns && !loading ? (
				!error && (
					<p className="text-sm text-muted-foreground text-center py-8">
						No data for this period.
					</p>
				)
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
									wrapperStyle={CHART_TOOLTIP_WRAPPER_STYLE}
									content={<VolumeStatusTooltipContent />}
								/>
								<Legend wrapperStyle={CHART_LEGEND_WRAPPER_STYLE} />
								{STATUS_SERIES.map((series) => (
									<Bar
										key={series.key}
										dataKey={series.key}
										stackId="status"
										fill={series.color}
										name={series.label}
									/>
								))}
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
									wrapperStyle={CHART_TOOLTIP_WRAPPER_STYLE}
									contentStyle={CHART_TOOLTIP_CONTENT_STYLE}
									formatter={(v) => formatDuration(Number(v ?? 0))}
								/>
								<Legend wrapperStyle={CHART_LEGEND_WRAPPER_STYLE} />
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
									contentStyle={CHART_TOOLTIP_CONTENT_STYLE}
									formatter={(v) => formatCost(Number(v ?? 0))}
								/>
								<Area
									type="monotone"
									dataKey="cumulative_cost"
									stroke={chartColor(3)}
									fill={chartColor(3)}
									fillOpacity={0.15}
									name="Cumulative cost (USD)"
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
									contentStyle={CHART_TOOLTIP_CONTENT_STYLE}
									formatter={(v) => (v == null ? "-" : `${v}%`)}
								/>
								<Line
									type="monotone"
									dataKey="success_rate"
									stroke={chartColor(1)}
									name="Success Rate"
									// Buckets without finished runs are gaps: a dot keeps
									// an isolated value visible between two gaps.
									dot={{ r: 2 }}
									strokeWidth={2}
									connectNulls={false}
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
