import { useLoaderData, useNavigation, Link } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import {
	createLoader,
	parseAsArrayOf,
	parseAsBoolean,
	parseAsString,
} from "nuqs/server";
import {
	useQueryStates,
	useQueryState,
	parseAsArrayOf as parseAsArrayOfClient,
	parseAsBoolean as parseAsBooleanClient,
	parseAsString as parseAsStringClient,
	parseAsStringLiteral,
} from "nuqs";
import { Info } from "lucide-react";
import type { RunResponse, StatsResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { CollapsibleSection } from "@/app/components/CollapsibleSection";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useRevalidateOnEvent } from "@/app/hooks/use-revalidate-on-event";
import { RunFilters } from "../runs/_components/RunFilters";
import { StatsCards } from "./_components/StatsCards";
import { StatsCharts } from "./_components/StatsCharts";
import { PERIODS, type Period } from "./_components/stats-chart-data";
import { countActiveFilters, toFilterParams } from "./stats-filters";
import { RecentRuns } from "./_components/RecentRuns";

export interface DashboardLoaderData {
	stats: StatsResponse;
	recentRuns: RunResponse[];
}

const filterParsers = {
	workflow: parseAsString.withDefault(""),
	status: parseAsString.withDefault(""),
	has_steps: parseAsBoolean.withDefault(true),
	label: parseAsArrayOf(parseAsString).withDefault([]),
	created_by: parseAsString.withDefault(""),
};

const loadFilters = createLoader(filterParsers);

export async function loader({ request }: LoaderFunctionArgs) {
	const filters = loadFilters(request);
	const filterParams = toFilterParams(filters);

	const runsParams = new URLSearchParams(filterParams);
	runsParams.set("page", "1");
	runsParams.set("per_page", "5");

	const statsQs = filterParams.toString();
	const statsUrl = statsQs ? `/stats?${statsQs}` : "/stats";

	const [statsRes, runsRes] = await Promise.all([
		api.get<StatsResponse>(statsUrl),
		api.get<RunResponse[]>(`/runs?${runsParams}`),
	]);
	return { stats: statsRes.data, recentRuns: runsRes.data };
}

export function Component() {
	const { stats, recentRuns } = useLoaderData() as DashboardLoaderData;
	const navigation = useNavigation();
	const isLoading = navigation.state === "loading";

	const [filters] = useQueryStates({
		workflow: parseAsStringClient.withDefault(""),
		status: parseAsStringClient.withDefault(""),
		has_steps: parseAsBooleanClient.withDefault(true),
		label: parseAsArrayOfClient(parseAsStringClient).withDefault([]),
		created_by: parseAsStringClient.withDefault(""),
	});

	const [period, setPeriod] = useQueryState(
		"history_period",
		parseAsStringLiteral(PERIODS).withDefault("7d"),
	);

	const activeCount = countActiveFilters(filters);

	useDocumentMeta({
		title: "Dashboard",
		description: "Overview of your workflow executions.",
	});
	useRevalidateOnEvent();

	return (
		<HeaderApp
			title="Dashboard"
			description="Overview of your workflow executions."
		>
			<div className="space-y-6">
				<RunFilters paginated={false} />
				<div className="space-y-6">
					{activeCount > 0 && (
						<div className="flex items-center gap-2 text-xs text-muted-foreground bg-muted/40 border border-border rounded px-3 py-1.5">
							<Info className="size-3.5 shrink-0" aria-hidden="true" />
							Stats filtered by active filters
						</div>
					)}
					<div
						aria-busy={isLoading}
						className={
							isLoading
								? "opacity-50 pointer-events-none transition-opacity"
								: ""
						}
					>
						{isLoading && (
							<span className="sr-only" aria-live="polite">
								Loading dashboard data
							</span>
						)}
						<div className="space-y-6">
							<CollapsibleSection
								storageKey="dashboard-overview"
								title="Overview"
								defaultOpen
								accent="var(--chart-1)"
								actions={
									<span className="text-xs text-muted-foreground">
										{activeCount > 0 ? "All time (filtered)" : "All time"}
									</span>
								}
							>
								<StatsCards stats={stats} />
							</CollapsibleSection>
							<CollapsibleSection
								storageKey="dashboard-trends"
								title="Trends"
								defaultOpen
								accent="var(--chart-2)"
								actions={
									<PeriodSelector
										period={period}
										onPeriodChange={(p) => setPeriod(p)}
									/>
								}
							>
								<StatsCharts
									filters={filters}
									period={period}
									refreshKey={stats}
								/>
							</CollapsibleSection>
							<CollapsibleSection
								storageKey="dashboard-recent"
								title="Recent Runs"
								defaultOpen
								accent="var(--chart-3)"
								actions={
									<Link
										to="/runs"
										className="text-xs font-medium text-primary hover:text-primary/80"
									>
										View All
									</Link>
								}
							>
								<RecentRuns runs={recentRuns} />
							</CollapsibleSection>
						</div>
					</div>
				</div>
			</div>
		</HeaderApp>
	);
}

function PeriodSelector({
	period,
	onPeriodChange,
}: {
	period: Period;
	onPeriodChange: (p: Period) => void;
}) {
	return (
		<div className="flex gap-1">
			{PERIODS.map((p) => (
				<button
					key={p}
					type="button"
					onClick={() => onPeriodChange(p)}
					className={`px-2.5 py-1 text-xs rounded-md transition-colors ${
						period === p
							? "bg-foreground text-background"
							: "bg-muted text-muted-foreground hover:bg-muted/80"
					}`}
				>
					{p}
				</button>
			))}
		</div>
	);
}
