import { api } from "@/app/lib/api";
import type { RunResponse } from "@/app/lib/types";

export function cancelRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/cancel`).then((res) => res.data);
}

export function retryRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/retry`).then((res) => res.data);
}

export function replayRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/replay`).then((res) => res.data);
}

export function approveRun(runId: string): Promise<RunResponse> {
	return api
		.post<RunResponse>(`/runs/${runId}/approve`)
		.then((res) => res.data);
}

export function rejectRun(runId: string): Promise<RunResponse> {
	return api.post<RunResponse>(`/runs/${runId}/reject`).then((res) => res.data);
}

export function submitStepInput(
	runId: string,
	stepId: string,
	value: unknown,
): Promise<RunResponse> {
	return api
		.post<RunResponse>(`/runs/${runId}/steps/${stepId}/input`, value)
		.then((res) => res.data);
}

export function rejectStepInput(
	runId: string,
	stepId: string,
	reason?: string,
): Promise<RunResponse> {
	return api
		.post<RunResponse>(`/runs/${runId}/steps/${stepId}/reject`, { reason })
		.then((res) => res.data);
}
