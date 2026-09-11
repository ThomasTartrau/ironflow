import type { RunResponse } from "@/app/lib/types";
import { RunsTable } from "@/app/routes/runs/_components/RunsTable";

interface RecentRunsProps {
	runs: RunResponse[];
}

export function RecentRuns({ runs }: RecentRunsProps) {
	return <RunsTable runs={runs} />;
}
