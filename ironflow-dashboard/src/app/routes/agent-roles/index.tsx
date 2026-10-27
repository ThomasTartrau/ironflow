import { useLoaderData } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import {
	useQueryStates,
	parseAsInteger,
	parseAsString,
	parseAsStringLiteral,
} from "nuqs";
import { api } from "@/app/lib/api";
import type { AgentRoleStatsResponse, StepResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Pagination } from "@/app/components/Pagination";
import {
	PERIODS,
	type Period,
} from "../dashboard/_components/stats-chart-data";
import { AgentRoleStatsTable } from "./_components/AgentRoleStatsTable";
import { AgentRoleStepsTable } from "./_components/AgentRoleStepsTable";

const PER_PAGE = 50;

const DESCRIPTION =
	"Cost, executions and failures per agent role, all workflows combined.";

interface AgentRolesLoaderData {
	stats: AgentRoleStatsResponse;
	steps: StepResponse[];
	meta: { page: number; per_page: number; total: number } | undefined;
}

function isPeriod(value: string | null): value is Period {
	return PERIODS.some((p) => p === value);
}

export async function loader({ request }: LoaderFunctionArgs) {
	const url = new URL(request.url);
	const rawPeriod = url.searchParams.get("period");
	const period: Period = isPeriod(rawPeriod) ? rawPeriod : "7d";
	const role = url.searchParams.get("role")?.trim() ?? "";

	const statsReq = api.get<AgentRoleStatsResponse>(
		`/stats/agent-roles?${new URLSearchParams({ period })}`,
	);
	if (!role) {
		const statsRes = await statsReq;
		return { stats: statsRes.data, steps: [], meta: undefined };
	}

	const stepsParams = new URLSearchParams({
		agent_role: role,
		page: url.searchParams.get("page") ?? "1",
		per_page: String(PER_PAGE),
	});
	const [statsRes, stepsRes] = await Promise.all([
		statsReq,
		api.get<StepResponse[]>(`/steps?${stepsParams}`),
	]);
	return { stats: statsRes.data, steps: stepsRes.data, meta: stepsRes.meta };
}

function PeriodSelector({
	period,
	onPeriodChange,
}: {
	period: Period;
	onPeriodChange: (p: Period) => void;
}) {
	return (
		<div className="flex gap-1">
			{PERIODS.map((p) => (
				<button
					key={p}
					type="button"
					onClick={() => onPeriodChange(p)}
					className={`px-2.5 py-1 text-xs rounded-md transition-colors ${
						period === p
							? "bg-foreground text-background"
							: "bg-muted text-muted-foreground hover:bg-muted/80"
					}`}
				>
					{p}
				</button>
			))}
		</div>
	);
}

export function Component() {
	const { stats, steps, meta } = useLoaderData() as AgentRolesLoaderData;
	useDocumentMeta({ title: "Agent Roles", description: DESCRIPTION });

	const [query, setQuery] = useQueryStates(
		{
			period: parseAsStringLiteral(PERIODS)
				.withDefault("7d")
				.withOptions({ shallow: false }),
			role: parseAsString.withDefault("").withOptions({ shallow: false }),
			page: parseAsInteger.withDefault(1).withOptions({ shallow: false }),
		},
		{ history: "push" },
	);

	const role = query.role.trim();
	const totalPages = meta ? Math.ceil(meta.total / meta.per_page) : 1;

	return (
		<HeaderApp title="Agent Roles" description={DESCRIPTION}>
			<div className="space-y-6">
				<PeriodSelector
					period={query.period}
					onPeriodChange={(period) => setQuery({ period })}
				/>
				<AgentRoleStatsTable
					roles={stats.roles}
					selectedRole={role}
					onSelectRole={(selected) => setQuery({ role: selected, page: 1 })}
				/>
				{role && (
					<div className="space-y-4">
						<h2 className="text-sm font-medium">{`Steps of role ${role}`}</h2>
						<AgentRoleStepsTable agentRole={role} steps={steps} />
						{meta && (
							<Pagination
								currentPage={query.page}
								totalPages={totalPages}
								total={meta.total}
								perPage={meta.per_page}
								onPageChange={(page) => setQuery({ page })}
							/>
						)}
					</div>
				)}
			</div>
		</HeaderApp>
	);
}
