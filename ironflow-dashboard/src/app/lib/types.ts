export type RunStatus =
	| "pending"
	| "running"
	| "completed"
	| "failed"
	| "retrying"
	| "cancelled"
	| "awaiting_approval";
export type StepKind =
	| "shell"
	| "http"
	| "agent"
	| "workflow"
	| "approval"
	| (string & {});
export type StepStatus =
	| "pending"
	| "running"
	| "completed"
	| "failed"
	| "skipped";

export type TriggerKind =
	| { kind: "manual" }
	| { kind: "webhook"; path: string }
	| { kind: "cron"; schedule: string }
	| { kind: "api" }
	| { kind: "retry"; parent_run_id: string }
	| { kind: "workflow" };

export interface RunResponse {
	id: string;
	workflow_name: string;
	status: RunStatus;
	trigger: TriggerKind;
	error: string | null;
	retry_count: number;
	max_retries: number;
	cost_usd: number;
	duration_ms: number;
	created_at: string;
	updated_at: string;
	started_at: string | null;
	completed_at: string | null;
}

export interface RunDetailResponse {
	run: RunResponse;
	steps: StepResponse[];
}

export interface StepResponse {
	id: string;
	run_id: string;
	name: string;
	kind: StepKind;
	position: number;
	status: StepStatus;
	input: Record<string, unknown> | null;
	output: Record<string, unknown> | null;
	error: string | null;
	duration_ms: number;
	cost_usd: number;
	input_tokens: number | null;
	output_tokens: number | null;
	created_at: string;
	updated_at: string;
	started_at: string | null;
	completed_at: string | null;
	dependencies: string[];
}

export interface StatsResponse {
	total_runs: number;
	completed_runs: number;
	failed_runs: number;
	cancelled_runs: number;
	active_runs: number;
	success_rate_percent: number;
	total_cost_usd: number;
	total_duration_ms: number;
}

export interface ApiResponse<T> {
	data: T;
	meta: { page: number; per_page: number; total: number } | null;
}

export interface CreateRunRequest {
	workflow: string;
	payload?: Record<string, unknown>;
}

export interface SubWorkflowDetail {
	name: string;
	description: string;
	source_code: string | null;
}

export interface WorkflowDetailResponse {
	name: string;
	description: string;
	source_code: string | null;
	sub_workflows: SubWorkflowDetail[];
}
