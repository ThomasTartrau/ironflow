import { useLoaderData } from "react-router";
import type { RunResponse, StatsResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useRevalidateOnEvent } from "@/app/hooks/use-revalidate-on-event";
import { StatsCards } from "./_components/StatsCards";
import { RecentRuns } from "./_components/RecentRuns";

export interface DashboardLoaderData {
	stats: StatsResponse;
	recentRuns: RunResponse[];
}

export async function loader() {
	const [statsRes, runsRes] = await Promise.all([
		api.get<StatsResponse>("/stats"),
		api.get<RunResponse[]>("/runs?page=1&per_page=5"),
	]);
	return { stats: statsRes.data, recentRuns: runsRes.data };
}

export function Component() {
	const { stats, recentRuns } = useLoaderData() as DashboardLoaderData;
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
				<StatsCards stats={stats} />
				<RecentRuns runs={recentRuns} />
			</div>
		</HeaderApp>
	);
}
