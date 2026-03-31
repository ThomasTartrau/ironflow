import { useState } from "react";
import { useLoaderData, useNavigate, Link } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import type {
	WorkflowDetailResponse,
	RunResponse,
	CreateRunRequest,
} from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { withToast } from "@/app/lib/api-toast";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { CodeBlock } from "@/app/components/CodeBlock";
import { StatusBadge } from "@/app/components/StatusBadge";
import { TimeAgo } from "@/app/components/TimeAgo";
import { Button } from "@/components/ui/button";
import { BackLink } from "@/app/components/BackLink";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { formatDuration, formatCost } from "@/app/lib/format";
import { Play } from "lucide-react";

interface LoaderData {
	workflow: WorkflowDetailResponse;
	recentRuns: RunResponse[];
}

export async function loader({ params }: LoaderFunctionArgs) {
	const [workflowRes, runsRes] = await Promise.all([
		api.get<WorkflowDetailResponse>(`/workflows/${params.name}`),
		api.get<RunResponse[]>(`/runs?workflow=${params.name}&page=1&per_page=10`),
	]);
	return { workflow: workflowRes.data, recentRuns: runsRes.data };
}

export function Component() {
	const { workflow, recentRuns } = useLoaderData() as LoaderData;
	const [loading, setLoading] = useState(false);
	const navigate = useNavigate();
	useDocumentMeta({
		title: workflow.name,
		description: workflow.description || "Workflow details and recent runs.",
	});

	const handleRun = () => {
		setLoading(true);
		const request: CreateRunRequest = {
			workflow: workflow.name,
			payload: {},
		};

		withToast(api.post<RunResponse>("/runs", request), {
			loading: "Starting run...",
			success: "Run started!",
			error: "Failed to start run",
		})
			.then((res) => navigate(`/runs/${res.data.id}`))
			.catch(() => {})
			.finally(() => setLoading(false));
	};

	return (
		<HeaderApp
			title={workflow.name}
			description={workflow.description || "No description provided."}
			titleItem={
				<Button
					className="w-full sm:w-fit gap-1.5"
					onClick={handleRun}
					disabled={loading}
				>
					<Play className="size-4" />
					{loading ? "Starting..." : "Run"}
				</Button>
			}
		>
			<div className="space-y-8">
				<BackLink to="/workflows" label="Back to Workflows" />

				{workflow.source_code && (
					<div className="space-y-3">
						<h2 className="text-base font-semibold tracking-tight">
							Handler source
						</h2>
						<CodeBlock code={workflow.source_code} />
					</div>
				)}

				{workflow.sub_workflows.length > 0 && (
					<div className="space-y-4">
						<h2 className="text-base font-semibold tracking-tight">
							Sub-workflows ({workflow.sub_workflows.length})
						</h2>
						{workflow.sub_workflows.map((sub) => (
							<div key={sub.name} className="space-y-2">
								<div className="flex items-center gap-2">
									<Link
										to={`/workflows/${sub.name}`}
										className="text-sm font-medium text-indigo-600 hover:underline"
									>
										{sub.name}
									</Link>
									{sub.description && (
										<span className="text-xs text-muted-foreground">
											{sub.description}
										</span>
									)}
								</div>
								{sub.source_code && <CodeBlock code={sub.source_code} />}
							</div>
						))}
					</div>
				)}

				<div className="space-y-3">
					<div className="flex items-center justify-between">
						<h2 className="text-base font-semibold tracking-tight">
							Recent runs ({recentRuns.length})
						</h2>
						<Link to={`/runs?workflow=${workflow.name}`}>
							<Button
								variant="ghost"
								size="sm"
								className="text-muted-foreground"
							>
								View all
							</Button>
						</Link>
					</div>
					{recentRuns.length === 0 ? (
						<div className="text-center py-12 text-muted-foreground border rounded-lg bg-muted/20">
							No runs yet for this workflow.
						</div>
					) : (
						<div className="rounded-lg border">
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Status</TableHead>
										<TableHead>Duration</TableHead>
										<TableHead>Cost</TableHead>
										<TableHead>Started</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{recentRuns.map((run) => (
										<TableRow key={run.id}>
											<TableCell>
												<Link to={`/runs/${run.id}`}>
													<StatusBadge status={run.status} />
												</Link>
											</TableCell>
											<TableCell>{formatDuration(run.duration_ms)}</TableCell>
											<TableCell>{formatCost(run.cost_usd)}</TableCell>
											<TableCell>
												<TimeAgo date={run.started_at ?? run.created_at} />
											</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						</div>
					)}
				</div>
			</div>
		</HeaderApp>
	);
}
