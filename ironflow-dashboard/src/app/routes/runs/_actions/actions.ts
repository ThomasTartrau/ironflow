import { api } from "@/app/lib/api";
import type { RunResponse } from "@/app/lib/types";

export function cancelRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/cancel`).then((res) => res.data);
}

export function retryRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/retry`).then((res) => res.data);
}
