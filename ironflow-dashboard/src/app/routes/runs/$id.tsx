import { useEffect, useRef } from "react";
import { useLoaderData, useRevalidator } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import type {
	RunDetailResponse,
	RunResponse,
	RunStatus,
	StepResponse,
} from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { StatCard } from "@/app/components/StatCard";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TriggerBadge } from "@/app/components/TriggerBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import { CollapsibleSection } from "@/app/components/CollapsibleSection";
import { RunActions } from "./_components/RunActions";
import { StepList } from "./_components/StepList";
import { StepFlow } from "./_components/StepFlow";
import { StepTimeline } from "./_components/StepTimeline";
import { BackLink } from "@/app/components/BackLink";
import { formatDuration, formatCost } from "@/app/lib/format";
import { Clock, DollarSign, RotateCcw, Calendar } from "lucide-react";

const POLL_INTERVAL_MS = 3000;

export async function loader({ params }: LoaderFunctionArgs) {
	const res = await api.get<RunDetailResponse>(`/runs/${params.id}`);
	return res.data;
}

function isRunActive(status: RunStatus): boolean {
	return status === "pending" || status === "running";
}

export function Component() {
	const { run, steps } = useLoaderData() as {
		run: RunResponse;
		steps: StepResponse[];
	};
	useDocumentMeta({
		title: `${run.workflow_name} · Run ${run.id.slice(0, 8)}`,
		description: `Run ${run.id} of workflow ${run.workflow_name}.`,
	});
	const revalidator = useRevalidator();
	const revalidatorRef = useRef(revalidator);
	revalidatorRef.current = revalidator;
	const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

	useEffect(() => {
		if (!isRunActive(run.status)) {
			return;
		}

		intervalRef.current = setInterval(() => {
			if (revalidatorRef.current.state === "idle") {
				revalidatorRef.current.revalidate();
			}
		}, POLL_INTERVAL_MS);

		return () => {
			if (intervalRef.current) {
				clearInterval(intervalRef.current);
			}
		};
	}, [run.status]);

	return (
		<HeaderApp
			title={run.workflow_name}
			description={`Run ${run.id}`}
			titleItem={
				<div className="flex items-center gap-2">
					<StatusBadge status={run.status} />
					<TriggerBadge trigger={run.trigger} />
					<RunActions run={run} />
				</div>
			}
		>
			<div className="space-y-6">
				<BackLink to="/runs" label="Back to Runs" />

				<div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
					<StatCard
						label="Started"
						value={run.started_at ? <TimeAgo date={run.started_at} /> : "-"}
						icon={Calendar}
					/>
					<StatCard
						label="Completed"
						value={run.completed_at ? <TimeAgo date={run.completed_at} /> : "-"}
						icon={Calendar}
					/>
					<StatCard
						label="Duration"
						value={formatDuration(run.duration_ms)}
						icon={Clock}
					/>
					<StatCard
						label="Cost"
						value={formatCost(run.cost_usd)}
						icon={DollarSign}
					/>
					<StatCard
						label="Retries"
						value={`${run.retry_count} / ${run.max_retries}`}
						icon={RotateCcw}
					/>
				</div>

				{run.error && (
					<div className="p-4 rounded-lg border border-red-200 bg-red-50">
						<div className="text-sm font-semibold text-red-600 mb-1">Error</div>
						<div className="text-sm text-red-700 whitespace-pre-wrap break-words">
							{run.error}
						</div>
					</div>
				)}

				<div className="space-y-3">
					<h2 className="text-base font-semibold tracking-tight">
						Steps ({steps.length})
					</h2>
					<CollapsibleSection
						storageKey="steps-timeline"
						title="Timeline"
						defaultOpen
					>
						<StepTimeline
							steps={steps}
							runStartedAt={run.started_at}
							runId={run.id}
						/>
					</CollapsibleSection>
					<CollapsibleSection storageKey="steps-flow" title="Flow">
						<StepFlow
							steps={steps}
							workflowName={run.workflow_name}
							runId={run.id}
						/>
					</CollapsibleSection>
					<StepList steps={steps} />
				</div>
			</div>
		</HeaderApp>
	);
}
