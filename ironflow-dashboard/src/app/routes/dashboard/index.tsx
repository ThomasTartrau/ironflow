import { useLoaderData, useNavigation } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import {
	createLoader,
	parseAsArrayOf,
	parseAsBoolean,
	parseAsString,
} from "nuqs/server";
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
				<div
					className={
						isLoading ? "opacity-50 pointer-events-none transition-opacity" : ""
					}
				>
					<StatsCards stats={stats} />
					<RecentRuns runs={recentRuns} />
				</div>
			</div>
		</HeaderApp>
	);
}
