import { useLoaderData } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import { useQueryState, parseAsString, debounce } from "nuqs";
import { api } from "@/app/lib/api";
import type { WorkflowDetailResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { useNavigate } from "react-router";

export async function loader({ request }: LoaderFunctionArgs) {
	const url = new URL(request.url);
	const name = url.searchParams.get("name") ?? "";

	const params = new URLSearchParams();
	if (name) params.set("name", name);

	const queryString = params.toString();
	const res = await api.get<string[]>(
		`/workflows${queryString ? `?${queryString}` : ""}`,
	);
	const workflows = await Promise.all(
		res.data.map((n) =>
			api.get<WorkflowDetailResponse>(`/workflows/${n}`).then((r) => r.data),
		),
	);
	return { workflows };
}

export function Component() {
	const { workflows } = useLoaderData() as {
		workflows: WorkflowDetailResponse[];
	};
	const navigate = useNavigate();
	const [nameFilter, setNameFilter] = useQueryState(
		"name",
		parseAsString.withDefault("").withOptions({
			shallow: false,
			limitUrlUpdates: debounce(300),
			history: "replace",
		}),
	);
	useDocumentMeta({
		title: "Workflows",
		description: "Registered workflow handlers in the engine.",
	});

	return (
		<HeaderApp
			title="Workflows"
			description="Registered workflow handlers in the engine."
		>
			<div className="space-y-6">
				<div className="flex flex-col gap-4 md:flex-row md:items-end">
					<div className="flex-1">
						<label htmlFor="filter-name" className="text-sm font-medium">
							Workflow Name
						</label>
						<Input
							id="filter-name"
							placeholder="Filter by workflow name..."
							value={nameFilter}
							onChange={(e) => setNameFilter(e.target.value || null)}
							className="mt-1"
						/>
					</div>
					{nameFilter && (
						<Button onClick={() => setNameFilter(null)} variant="outline">
							Remove all filters
						</Button>
					)}
				</div>

				{workflows.length === 0 ? (
					<div className="text-center py-16 text-muted-foreground border rounded-lg bg-muted/20">
						No workflows registered.
					</div>
				) : (
					<div className="rounded-lg border">
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>Name</TableHead>
									<TableHead>Description</TableHead>
									<TableHead>Sub-workflows</TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{workflows.map((wf) => (
									<TableRow
										key={wf.name}
										className="cursor-pointer hover:bg-muted/50"
										onClick={() => navigate(`/workflows/${wf.name}`)}
									>
										<TableCell className="font-medium">{wf.name}</TableCell>
										<TableCell className="text-sm text-muted-foreground max-w-md truncate">
											{wf.description || "-"}
										</TableCell>
										<TableCell>
											{wf.sub_workflows.length > 0 ? (
												<div className="flex flex-wrap gap-1">
													{wf.sub_workflows.map((sub) => (
														<Badge
															key={sub.name}
															variant="outline"
															className="text-xs"
														>
															{sub.name}
														</Badge>
													))}
												</div>
											) : (
												<span className="text-sm text-muted-foreground">-</span>
											)}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					</div>
				)}
			</div>
		</HeaderApp>
	);
}
