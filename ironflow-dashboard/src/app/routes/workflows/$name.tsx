import { useLoaderData, useNavigate, Link } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import type { WorkflowDetailResponse, RunResponse } from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { CodeBlock } from "@/app/components/CodeBlock";
import { Breadcrumb } from "@/app/components/Breadcrumb";
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
					<Breadcrumb
						items={[
							{ label: "Workflows", to: "/workflows" },
							{ label: workflow.name },
						]}
					/>
					{workflow.version !== "unversioned" && (
						<Badge variant="outline" className="gap-1 font-mono text-xs">
							<Tag className="size-3" aria-hidden="true" />
							{workflow.version}
						</Badge>
					)}
				</div>

				<div className="space-y-3">
					<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground border-l-2 border-primary pl-3">
						Overview
					</h2>
					<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
						<div className="rounded-[var(--radius)] border p-3">
							<p className="text-xs text-muted-foreground mb-1">Description</p>
							<p className="text-sm">
								{workflow.description || "No description provided."}
							</p>
						</div>
						<div className="rounded-[var(--radius)] border p-3">
							<p className="text-xs text-muted-foreground mb-1">Version</p>
							<p className="text-sm font-mono">{workflow.version}</p>
						</div>
						<div className="rounded-[var(--radius)] border p-3">
							<p className="text-xs text-muted-foreground mb-1">Recent runs</p>
							<p className="text-sm">{recentRuns.length}</p>
						</div>
					</div>
				</div>

				{workflow.source_code && (
					<div className="space-y-3">
						<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground border-l-2 border-primary pl-3">
							Handler source
						</h2>
						<CodeBlock code={workflow.source_code} />
					</div>
				)}

				{workflow.sub_workflows.length > 0 && (
					<div className="space-y-4">
						<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground border-l-2 border-primary pl-3">
							Sub-workflows ({workflow.sub_workflows.length})
						</h2>
						{workflow.sub_workflows.map((sub) => (
							<div key={sub.name} className="space-y-2">
								<div className="flex items-center gap-2">
									<Link
										to={`/workflows/${sub.name}`}
										className="text-sm font-medium text-primary hover:text-primary/80 underline underline-offset-2"
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
						<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground border-l-2 border-primary pl-3">
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
