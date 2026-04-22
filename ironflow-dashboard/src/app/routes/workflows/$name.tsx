import { useLoaderData, useNavigate, Link } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import type { WorkflowDetailResponse, RunResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { CodeBlock } from "@/app/components/CodeBlock";
import { BackLink } from "@/app/components/BackLink";
import { RunsTable } from "@/app/routes/runs/_components/RunsTable";
import { RunDialog } from "./_components/RunDialog";
import { Button } from "@/components/ui/button";
import { Tag } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { useAppSelector } from "@/app/store";

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
	const navigate = useNavigate();
	const auth = useAppSelector((state) => state.auth);
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;
	useDocumentMeta({
		title: workflow.name,
		description: workflow.description || "Workflow details and recent runs.",
	});

	return (
		<HeaderApp
			title={workflow.name}
			description={workflow.description || "No description provided."}
			titleItem={
				isAdmin ? (
					<RunDialog
						workflow={workflow}
						onCreated={(id) => navigate(`/runs/${id}`)}
					/>
				) : undefined
			}
		>
			<div className="space-y-8">
				<div className="flex items-center justify-between">
					<BackLink to="/workflows" label="Back to Workflows" />
					{workflow.version !== "unversioned" && (
						<Badge variant="outline" className="gap-1 font-mono text-xs">
							<Tag className="size-3" />
							{workflow.version}
						</Badge>
					)}
				</div>

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
					<RunsTable runs={recentRuns} />
				</div>
			</div>
		</HeaderApp>
	);
}
