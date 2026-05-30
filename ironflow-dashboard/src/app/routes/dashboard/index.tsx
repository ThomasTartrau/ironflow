import { useLoaderData, useNavigation } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import {
	createLoader,
	parseAsArrayOf,
	parseAsBoolean,
	parseAsString,
} from "nuqs/server";
import {
	useQueryStates,
	parseAsArrayOf as parseAsArrayOfClient,
	parseAsBoolean as parseAsBooleanClient,
	parseAsString as parseAsStringClient,
} from "nuqs";
import { Info } from "lucide-react";
import type { RunResponse, StatsResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useRevalidateOnEvent } from "@/app/hooks/use-revalidate-on-event";
import { RunFilters } from "../runs/_components/RunFilters";
import { StatsCards } from "./_components/StatsCards";
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
};

const loadFilters = createLoader(filterParsers);

function toApiParams(filters: {
	workflow: string;
	status: string;
	has_steps: boolean;
	label: string[];
}): URLSearchParams {
	const params = new URLSearchParams();
	if (filters.workflow) params.set("workflow", filters.workflow);
	if (filters.status) params.set("status", filters.status);
	if (filters.has_steps) params.set("has_steps", "true");
	if (filters.label.length > 0) params.set("label", filters.label.join(","));
	return params;
}

export async function loader({ request }: LoaderFunctionArgs) {
	const filters = loadFilters(request);
	const filterParams = toApiParams(filters);

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
	});

	const activeCount = [
		filters.workflow,
		filters.status,
		!filters.has_steps ? "has_steps" : "",
		filters.label.length > 0 ? "label" : "",
	].filter(Boolean).length;

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
				<RunFilters />
				<section className="border-t border-border pt-6 space-y-6">
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
						<StatsCards stats={stats} />
						<RecentRuns runs={recentRuns} />
					</div>
				</section>
			</div>
		</HeaderApp>
	);
}
