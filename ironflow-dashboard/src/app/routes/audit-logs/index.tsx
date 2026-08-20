import { useLoaderData } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import { useQueryStates, parseAsInteger, parseAsString } from "nuqs";
import { api } from "@/app/lib/api";
import type { AuditLogEntry, UserResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Pagination } from "@/app/components/Pagination";
import { AuditLogsTable } from "./_components/AuditLogsTable";
import { AuditLogsFilters } from "./_components/AuditLogsFilters";

const PER_PAGE = 50;

const UUID_RE =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function toIso8601(datetimeLocal: string): string | null {
	if (!datetimeLocal) return null;
	const d = new Date(datetimeLocal);
	if (Number.isNaN(d.getTime())) return null;
	return d.toISOString();
}

interface AuditLogsLoaderData {
	entries: AuditLogEntry[];
	meta: { page: number; per_page: number; total: number } | undefined;
	users: UserResponse[];
}

export async function loader({ request }: LoaderFunctionArgs) {
	const url = new URL(request.url);
	const params = new URLSearchParams();

	const page = url.searchParams.get("page") ?? "1";
	params.set("page", page);
	params.set("per_page", String(PER_PAGE));

	const eventType = url.searchParams.get("event_type");
	if (eventType) params.set("event_type", eventType);

	const runId = url.searchParams.get("run_id");
	if (runId && UUID_RE.test(runId)) {
		params.set("run_id", runId);
	}

	const from = url.searchParams.get("from");
	const fromIso = toIso8601(from ?? "");
	if (fromIso) params.set("from", fromIso);

	const to = url.searchParams.get("to");
	const toIso = toIso8601(to ?? "");
	if (toIso) params.set("to", toIso);

	const [logsRes, usersRes] = await Promise.all([
		api.get<AuditLogEntry[]>(`/audit-logs?${params}`),
		api
			.get<UserResponse[]>("/users?page=1&per_page=100")
			.catch(() => ({ data: [] as UserResponse[] })),
	]);

	return {
		entries: logsRes.data,
		meta: logsRes.meta,
		users: usersRes.data,
	};
}

export function Component() {
	const { entries, meta, users } = useLoaderData() as AuditLogsLoaderData;
	useDocumentMeta({
		title: "Audit Logs",
		description: "Review persisted domain events for compliance and debugging.",
	});

	const userMap = new Map(users.map((u) => [u.id, u]));

	const [queryFilters, setQueryFilters] = useQueryStates(
		{
			page: parseAsInteger.withDefault(1).withOptions({ shallow: false }),
			event_type: parseAsString.withDefault("").withOptions({ shallow: false }),
			run_id: parseAsString.withDefault("").withOptions({ shallow: false }),
			from: parseAsString.withDefault("").withOptions({ shallow: false }),
			to: parseAsString.withDefault("").withOptions({ shallow: false }),
		},
		{ history: "push" },
	);

	const currentPage = queryFilters.page;
	const totalPages = meta ? Math.ceil(meta.total / meta.per_page) : 1;

	return (
		<HeaderApp
			title="Audit Logs"
			description="Review persisted domain events for compliance and debugging."
		>
			<div className="space-y-4">
				<AuditLogsFilters
					filters={queryFilters}
					onFilterChange={(updates) => setQueryFilters({ ...updates, page: 1 })}
					onReset={() =>
						setQueryFilters({
							event_type: null,
							run_id: null,
							from: null,
							to: null,
							page: null,
						})
					}
				/>
				<AuditLogsTable entries={entries} userMap={userMap} />
				{meta && (
					<Pagination
						currentPage={currentPage}
						totalPages={totalPages}
						total={meta.total}
						perPage={meta.per_page}
						onPageChange={(page) => setQueryFilters({ page })}
					/>
				)}
			</div>
		</HeaderApp>
	);
}
