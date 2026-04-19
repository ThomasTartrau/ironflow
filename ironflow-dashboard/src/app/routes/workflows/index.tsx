import { ChevronRight, FolderClosed, FolderOpen, Workflow } from "lucide-react";
import { useMemo } from "react";
import { useLoaderData, useNavigate } from "react-router";
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
const UNCATEGORIZED_API_FILTER = "__uncategorized__";

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
		params.set("category", UNCATEGORIZED_API_FILTER);
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
}

function TreeNodeRow({ node, depth, openFolders, onToggle }: TreeNodeRowProps) {
	const navigate = useNavigate();
	const isOpen = openFolders.has(node.path);
	const descendantCount = countDescendantWorkflows(node);

	return (
		<Collapsible open={isOpen} onOpenChange={(v) => onToggle(node.path, v)}>
			<CollapsibleTrigger
				className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/50 transition-colors"
				style={{ paddingLeft: `${depth * 16 + 12}px` }}
			>
				<ChevronRight
					className={`size-4 text-muted-foreground transition-transform ${
						isOpen ? "rotate-90" : ""
					}`}
				/>
				{isOpen ? (
					<FolderOpen className="size-4 text-muted-foreground" />
				) : (
					<FolderClosed className="size-4 text-muted-foreground" />
				)}
				<span className="font-medium text-sm">{node.name}</span>
				<span className="ml-auto text-xs text-muted-foreground">
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
						/>
					))}
					{node.workflows.map((wf) => (
						<button
							type="button"
							key={wf.name}
							onClick={() => navigate(`/workflows/${wf.name}`)}
							className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-muted/50 transition-colors"
							style={{ paddingLeft: `${(depth + 1) * 16 + 12}px` }}
						>
							<span className="size-4 shrink-0" aria-hidden />
							<Workflow className="size-4 text-muted-foreground" />
							<span className="font-mono">{wf.name}</span>
						</button>
					))}
				</div>
			</CollapsibleContent>
		</Collapsible>
	);
}

export function Component() {
	const { workflows } = useLoaderData() as { workflows: WorkflowSummary[] };

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
						className="flex items-center gap-2 text-sm select-none cursor-pointer pb-2"
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
						>
							Remove all filters
						</Button>
					)}
				</div>

				{workflows.length === 0 ? (
					<div className="text-center py-16 text-muted-foreground border rounded-lg bg-muted/20">
						No workflows registered.
					</div>
				) : (
					<>
						<div className="flex justify-end gap-2">
							<Button variant="ghost" size="sm" onClick={handleExpandAll}>
								Expand all
							</Button>
							<Button variant="ghost" size="sm" onClick={handleCollapseAll}>
								Collapse all
							</Button>
						</div>
						<div className="rounded-lg border divide-y">
							{[...tree.children.values()].map((child) => (
								<TreeNodeRow
									key={child.path}
									node={child}
									depth={0}
									openFolders={openFolders}
									onToggle={handleToggle}
								/>
							))}
						</div>
					</>
				)}
			</div>
		</HeaderApp>
	);
}
