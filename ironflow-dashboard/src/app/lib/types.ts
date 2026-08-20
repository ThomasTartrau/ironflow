/**
 * Re-exports from the auto-generated OpenAPI types.
 *
 * Run `pnpm generate:types` to regenerate `types.generated.ts` from the backend.
 * Import types from this file, NOT from `types.generated.ts` directly.
 */
export type { components, operations, paths } from "./types.generated";
import type { components } from "./types.generated";

// -- Enums / union types --
export type RunStatus = components["schemas"]["RunStatus"];
export type StepStatus = components["schemas"]["StepStatus"];
export type ApiKeyScope = components["schemas"]["ApiKeyScope"];
export type TriggerKind = components["schemas"]["TriggerKind"];
export type CreatedByKind = components["schemas"]["CreatedByKind"];

// StepKind is a plain string in the OpenAPI spec (built-in + custom values)
export type StepKind = string;

// -- Events (SSE) --
export type EventKind = components["schemas"]["EventKind"];
export type Event = components["schemas"]["Event"];
/** Map from event type discriminant to the matching payload variant. */
export type EventPayload<K extends EventKind> = Extract<Event, { type: K }>;

// -- Run --
export type RunResponse = components["schemas"]["RunResponse"];
export type CreatedBy = components["schemas"]["CreatedBy"];
export type RunDetailResponse = components["schemas"]["RunDetailResponse"];
export type CreateRunRequest = components["schemas"]["CreateRunRequest"];

// -- Step --
export type StepResponse = components["schemas"]["StepResponse"];

// -- Artifacts --
export type ArtifactResponse = components["schemas"]["ArtifactResponse"];

// -- Stats --
export type StatsResponse = components["schemas"]["StatsResponse"];

// -- Users --
export type UserResponse = components["schemas"]["UserResponse"];
export type CreateUserRequest = components["schemas"]["CreateUserRequest"];

// -- Auth --
export type MeResponse = components["schemas"]["MeResponse"];

// -- API Keys --
export type ApiKeyResponse = components["schemas"]["ApiKeyResponse"];
export type CreateApiKeyRequest = components["schemas"]["CreateApiKeyRequest"];
export type CreateApiKeyResponse =
	components["schemas"]["CreateApiKeyResponse"];
export type ScopeEntry = components["schemas"]["ScopeEntry"];

// -- Workflows --
export type WorkflowSummary = components["schemas"]["WorkflowSummary"];
export type WorkflowDetailResponse =
	components["schemas"]["WorkflowDetailResponse"];
export type SubWorkflowDetail = components["schemas"]["SubWorkflowDetail"];

// -- Audit Logs --
export type AuditLogEntry = components["schemas"]["AuditLogEntry"];
export type ListAuditLogsQuery = components["schemas"]["ListAuditLogsQuery"];

// -- Secret key management --
export type KeyVersionsResponse = components["schemas"]["KeyVersionsResponse"];
export type RotateSecretsRequest =
	components["schemas"]["RotateSecretsRequest"];
export type RotateSecretsResponse =
	components["schemas"]["RotateSecretsResponse"];

// -- Secrets --
export type SecretResponse = {
	id: string;
	key: string;
	created_at: string;
	updated_at: string;
};

export type SetSecretRequest = {
	key: string;
	value: string;
};

export type UpdateSecretRequest = {
	value: string;
};

export type ListSecretsQuery = {
	prefix?: string;
	page?: number;
	per_page?: number;
};

// -- Response envelope (kept manually - generic wrapper not in OpenAPI) --
export interface ApiResponse<T> {
	data: T;
	meta: { page: number; per_page: number; total: number } | null;
}
