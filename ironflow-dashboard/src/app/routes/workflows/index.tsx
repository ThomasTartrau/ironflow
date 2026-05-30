import { ChevronRight, FolderClosed, FolderOpen, Workflow } from "lucide-react";
import { useMemo } from "react";
import { useLoaderData, useNavigate, useParams } from "react-router";
import type { LoaderFunctionArgs } from "react-router";
import { debounce, parseAsArrayOf, parseAsString, useQueryState } from "nuqs";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { api } from "@/app/lib/api";
import type { WorkflowSummary } from "@/app/lib/types";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";

const UNCATEGORIZED_KEY = "__uncategorized__";

interface TreeNode {
	name: string;
	path: string;
	children: Map<string, TreeNode>;
	workflows: WorkflowSummary[];
}

function createNode(name: string, path: string): TreeNode {
	return { name, path, children: new Map(), workflows: [] };
}

function buildTree(workflows: WorkflowSummary[]): TreeNode {
	const root = createNode("", "");

	for (const wf of workflows) {
		if (!wf.category) {
			let bucket = root.children.get(UNCATEGORIZED_KEY);
			if (!bucket) {
				bucket = createNode("Uncategorized", UNCATEGORIZED_KEY);
				root.children.set(UNCATEGORIZED_KEY, bucket);
			}
			bucket.workflows.push(wf);
			continue;
		}

		const segments = wf.category.split("/");
		let cursor = root;
		let accumulatedPath = "";
		for (const segment of segments) {
			accumulatedPath = accumulatedPath
				? `${accumulatedPath}/${segment}`
				: segment;
			let next = cursor.children.get(segment);
			if (!next) {
				next = createNode(segment, accumulatedPath);
				cursor.children.set(segment, next);
			}
			cursor = next;
		}
		cursor.workflows.push(wf);
	}

	sortNode(root);
	return root;
}

function sortNode(node: TreeNode): void {
	node.children = new Map(
		[...node.children.entries()].sort(([a], [b]) => {
			if (a === UNCATEGORIZED_KEY) return 1;
			if (b === UNCATEGORIZED_KEY) return -1;
			return a.localeCompare(b);
		}),
	);
	node.workflows.sort((a, b) => a.name.localeCompare(b.name));
	for (const child of node.children.values()) {
		sortNode(child);
	}
}

function collectAllFolderPaths(node: TreeNode, acc: string[]): void {
	for (const child of node.children.values()) {
		acc.push(child.path);
		collectAllFolderPaths(child, acc);
	}
}

function countDescendantWorkflows(node: TreeNode): number {
	let count = node.workflows.length;
	for (const child of node.children.values()) {
		count += countDescendantWorkflows(child);
	}
	return count;
}

export async function loader({ request }: LoaderFunctionArgs) {
	const url = new URL(request.url);
	const name = url.searchParams.get("name") ?? "";
	const category = url.searchParams.get("category") ?? "";
	const uncategorizedOnly = url.searchParams.get("uncategorized") === "1";

	const params = new URLSearchParams();
	if (name) params.set("name", name);
	if (uncategorizedOnly) {
		params.set("category", UNCATEGORIZED_KEY);
	} else if (category) {
		params.set("category", category);
	}

	const queryString = params.toString();
	const res = await api.get<WorkflowSummary[]>(
		`/workflows${queryString ? `?${queryString}` : ""}`,
	);
	return { workflows: res.data };
}

interface TreeNodeRowProps {
	node: TreeNode;
	depth: number;
	openFolders: Set<string>;
	onToggle: (path: string, open: boolean) => void;
	currentWorkflowName: string | undefined;
}

function TreeNodeRow({
	node,
	depth,
	openFolders,
	onToggle,
	currentWorkflowName,
}: TreeNodeRowProps) {
	const navigate = useNavigate();
	const isOpen = openFolders.has(node.path);
	const descendantCount = countDescendantWorkflows(node);

	return (
		<Collapsible open={isOpen} onOpenChange={(v) => onToggle(node.path, v)}>
			<CollapsibleTrigger
				className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-accent hover:text-accent-foreground transition-colors"
				style={{ "--tree-depth": depth } as React.CSSProperties}
				aria-label={`${node.name} folder, ${descendantCount} ${descendantCount === 1 ? "workflow" : "workflows"}`}
			>
				<span className="pl-[calc(var(--tree-depth,0)*1rem)] contents" />
				<ChevronRight
					aria-hidden="true"
					className={`size-4 text-muted-foreground transition-transform duration-200 ${
						isOpen ? "rotate-90" : ""
					}`}
				/>
				{isOpen ? (
					<FolderOpen
						aria-hidden="true"
						className="size-4 text-muted-foreground"
					/>
				) : (
					<FolderClosed
						aria-hidden="true"
						className="size-4 text-muted-foreground"
					/>
				)}
				<span className="font-medium text-sm">{node.name}</span>
				<span className="ml-auto text-xs text-muted-foreground tabular-nums">
					{descendantCount} {descendantCount === 1 ? "workflow" : "workflows"}
				</span>
			</CollapsibleTrigger>
			<CollapsibleContent>
				<div>
					{[...node.children.values()].map((child) => (
						<TreeNodeRow
							key={child.path}
							node={child}
							depth={depth + 1}
							openFolders={openFolders}
							onToggle={onToggle}
							currentWorkflowName={currentWorkflowName}
						/>
					))}
					{node.workflows.map((wf) => {
						const isActive = wf.name === currentWorkflowName;
						return (
							<button
								type="button"
								key={wf.name}
								onClick={() => navigate(`/workflows/${wf.name}`)}
								aria-label={`Open workflow ${wf.name}`}
								data-active={isActive}
								className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground transition-colors data-[active=true]:bg-accent/70"
								style={{ "--tree-depth": depth + 1 } as React.CSSProperties}
							>
								<span className="pl-[calc(var(--tree-depth,0)*1rem)] contents" />
								<span className="size-4 shrink-0" aria-hidden="true" />
								<Workflow
									aria-hidden="true"
									className="size-4 text-muted-foreground"
								/>
								<span className="font-mono text-sm">{wf.name}</span>
								{wf.version !== "unversioned" && (
									<span className="ml-auto text-xs font-mono text-muted-foreground tabular-nums">
										{wf.version}
									</span>
								)}
							</button>
						);
					})}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}

export function Component() {
	const { workflows } = useLoaderData() as { workflows: WorkflowSummary[] };
	const params = useParams();
	const currentWorkflowName = params.name;

	const [nameFilter, setNameFilter] = useQueryState(
		"name",
		parseAsString.withDefault("").withOptions({
			shallow: false,
			limitUrlUpdates: debounce(300),
			history: "replace",
		}),
	);
	const [categoryFilter, setCategoryFilter] = useQueryState(
		"category",
		parseAsString.withDefault("").withOptions({
			shallow: false,
			history: "replace",
		}),
	);
	const [uncategorizedOnly, setUncategorizedOnly] = useQueryState(
		"uncategorized",
		parseAsString.withDefault("").withOptions({
			shallow: false,
			history: "replace",
		}),
	);
	const isUncategorizedOnly = uncategorizedOnly === "1";
	const [openFoldersArr, setOpenFoldersArr] = useQueryState(
		"expanded",
		parseAsArrayOf(parseAsString).withDefault([]).withOptions({
			history: "replace",
		}),
	);

	const tree = useMemo(() => buildTree(workflows), [workflows]);
	const openFolders = useMemo(() => new Set(openFoldersArr), [openFoldersArr]);

	useDocumentMeta({
		title: "Workflows",
		description: "Registered workflow handlers in the engine.",
	});

	const handleToggle = (path: string, open: boolean) => {
		const next = new Set(openFolders);
		if (open) {
			next.add(path);
		} else {
			next.delete(path);
		}
		setOpenFoldersArr([...next]);
	};

	const handleExpandAll = () => {
		const all: string[] = [];
		collectAllFolderPaths(tree, all);
		setOpenFoldersArr(all);
	};

	const handleCollapseAll = () => {
		setOpenFoldersArr([]);
	};

	const hasFilters = Boolean(
		nameFilter || categoryFilter || isUncategorizedOnly,
	);

	const totalWorkflows = workflows.length;

	return (
		<HeaderApp
			title="Workflows"
			description="Registered workflow handlers in the engine."
		>
			<div className="space-y-4">
				<div className="rounded-[var(--radius)] border bg-card px-4 py-3 flex flex-col gap-4 md:flex-row md:items-center">
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
					<div className="flex-1">
						<label htmlFor="filter-category" className="text-sm font-medium">
							Category
						</label>
						<Input
							id="filter-category"
							placeholder="e.g. data/etl"
							value={categoryFilter}
							onChange={(e) => setCategoryFilter(e.target.value || null)}
							className="mt-1 font-mono"
							disabled={isUncategorizedOnly}
						/>
					</div>
					<label
						htmlFor="filter-uncategorized-only"
						className="flex items-center gap-2 text-sm select-none cursor-pointer self-end pb-[0.1875rem]"
					>
						<Checkbox
							id="filter-uncategorized-only"
							checked={isUncategorizedOnly}
							onCheckedChange={(checked) => {
								const next = checked === true;
								setUncategorizedOnly(next ? "1" : null);
								if (next) {
									setCategoryFilter(null);
								}
							}}
						/>
						Uncategorized only
					</label>
					{hasFilters && (
						<Button
							onClick={() => {
								setNameFilter(null);
								setCategoryFilter(null);
								setUncategorizedOnly(null);
							}}
							variant="outline"
							size="sm"
						>
							Remove all filters
						</Button>
					)}
				</div>

				{workflows.length === 0 ? (
					<div className="flex flex-col items-center justify-center gap-4 py-20 border border-dashed rounded-[var(--radius)] bg-muted/20">
						<Workflow
							className="size-10 text-muted-foreground/40"
							aria-hidden="true"
						/>
						{hasFilters ? (
							<>
								<p className="text-sm font-medium text-muted-foreground">
									No workflows match your filters.
								</p>
								<Button
									variant="outline"
									size="sm"
									onClick={() => {
										setNameFilter(null);
										setCategoryFilter(null);
										setUncategorizedOnly(null);
									}}
								>
									Remove filters
								</Button>
							</>
						) : (
							<>
								<p className="text-sm font-medium text-muted-foreground">
									No workflows registered.
								</p>
								<p className="text-xs text-muted-foreground">
									Register a workflow handler in your engine to see it here.
								</p>
							</>
						)}
					</div>
				) : (
					<div className="rounded-[var(--radius)] border overflow-hidden">
						<div className="flex items-center justify-between px-3 py-2 border-b bg-muted/30">
							<span className="text-xs text-muted-foreground tabular-nums">
								{totalWorkflows}{" "}
								{totalWorkflows === 1 ? "workflow" : "workflows"}
							</span>
							<div className="flex gap-1">
								<Button
									variant="ghost"
									size="sm"
									onClick={handleExpandAll}
									className="h-7 text-xs px-2"
								>
									Expand all
								</Button>
								<Button
									variant="ghost"
									size="sm"
									onClick={handleCollapseAll}
									className="h-7 text-xs px-2"
								>
									Collapse all
								</Button>
							</div>
						</div>
						<div className="divide-y">
							{[...tree.children.values()].map((child) => (
								<TreeNodeRow
									key={child.path}
									node={child}
									depth={0}
									openFolders={openFolders}
									onToggle={handleToggle}
									currentWorkflowName={currentWorkflowName}
								/>
							))}
						</div>
					</div>
				)}
			</div>
		</HeaderApp>
	);
}
