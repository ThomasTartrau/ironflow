import { api } from "@/app/lib/api";
import type { WorkflowPauseResponse } from "@/app/lib/types";

/** Pause a workflow: workers stop picking its queued runs. */
export function pauseWorkflow(name: string): Promise<WorkflowPauseResponse> {
	return api
		.post<WorkflowPauseResponse>(`/workflows/${name}/pause`)
		.then((res) => res.data);
}

/** Resume a paused workflow: workers pick its queued runs again. */
export function resumeWorkflow(name: string): Promise<WorkflowPauseResponse> {
	return api
		.post<WorkflowPauseResponse>(`/workflows/${name}/resume`)
		.then((res) => res.data);
}
