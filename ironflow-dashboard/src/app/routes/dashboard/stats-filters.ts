/** Run filters shared by the dashboard stats, trends and recent runs. */
export interface DashboardFilters {
	workflow: string;
	status: string;
	has_steps: boolean;
	label: string[];
	created_by: string;
}

/**
 * API query params for the dashboard filters.
 *
 * `/stats`, `/stats/history` and `/runs` accept the same filter params, so
 * every dashboard request uses this single mapping.
 */
export function toFilterParams(filters: DashboardFilters): URLSearchParams {
	const params = new URLSearchParams();
	if (filters.workflow) params.set("workflow", filters.workflow);
	if (filters.status) params.set("status", filters.status);
	if (filters.has_steps) params.set("has_steps", "true");
	if (filters.label.length > 0) params.set("label", filters.label.join(","));
	if (filters.created_by) params.set("created_by", filters.created_by);
	return params;
}

/** Number of filters that differ from their default value. */
export function countActiveFilters(filters: DashboardFilters): number {
	return [
		filters.workflow,
		filters.status,
		!filters.has_steps ? "has_steps" : "",
		filters.label.length > 0 ? "label" : "",
		filters.created_by,
	].filter(Boolean).length;
}
