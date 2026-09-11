import {
	AlertTriangle,
	Package,
	Copy,
	Check,
	ExternalLink,
	User,
	Tag,
	XCircle,
} from "lucide-react";
import { useState } from "react";
import type { LoaderFunctionArgs } from "react-router";
import { useLoaderData } from "react-router";
import {
	parseAsString,
	parseAsArrayOf,
	useQueryState,
	useQueryStates,
} from "nuqs";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { api } from "@/app/lib/api";
import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { MultiSelect } from "@/components/ui/multi-select";
import {
	Sheet,
	SheetContent,
	SheetHeader,
	SheetTitle,
	SheetDescription,
} from "@/components/ui/sheet";

interface TemplateEntry {
	name: string;
	description: string;
	category?: string;
	repo: string;
	min_ironflow_version?: string;
	authors?: string[];
	latest_version?: string;
}

interface RegistryResponse {
	templates: TemplateEntry[];
	ironflow_version: string;
}

interface LoaderData {
	templates: TemplateEntry[];
	ironflow_version: string;
	error?: string;
}

export async function loader(_args: LoaderFunctionArgs): Promise<LoaderData> {
	try {
		const res = await api.get<RegistryResponse>("/templates/registry");
		return {
			templates: res.data.templates,
			ironflow_version: res.data.ironflow_version,
		};
	} catch {
		return {
			templates: [],
			ironflow_version: "0.0.0",
			error:
				"Could not reach the template registry. Check your IRONFLOW_REGISTRY_URL configuration or try again later.",
		};
	}
}

function isVersionIncompatible(
	required: string | undefined,
	current: string,
): boolean {
	if (!required) return false;
	const parse = (v: string) => v.split(".").map(Number);
	const [rMajor, rMinor = 0, rPatch = 0] = parse(required);
	const [cMajor, cMinor = 0, cPatch = 0] = parse(current);
	if (cMajor !== rMajor) return cMajor < rMajor;
	if (cMinor !== rMinor) return cMinor < rMinor;
	return cPatch < rPatch;
}

function CopyButton({ text }: { text: string }) {
	const [copied, setCopied] = useState(false);

	const handleCopy = (e: React.MouseEvent) => {
		e.stopPropagation();
		navigator.clipboard.writeText(`ironflow template add ${text}`);
		setCopied(true);
		setTimeout(() => setCopied(false), 2000);
	};

	return (
		<button
			type="button"
			onClick={handleCopy}
			className="flex w-full items-center gap-1.5 rounded-md border px-3 py-1.5 text-xs font-mono bg-muted hover:bg-accent transition-colors"
			title="Copy install command"
		>
			{copied ? (
				<Check className="h-3.5 w-3.5 shrink-0 text-green-500" />
			) : (
				<Copy className="h-3.5 w-3.5 shrink-0" />
			)}
			<span className="truncate">ironflow template add {text}</span>
		</button>
	);
}

function TemplateDetailSheet({
	template,
	ironflowVersion,
	open,
	onOpenChange,
}: {
	template: TemplateEntry | null;
	ironflowVersion: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}) {
	if (!template) return null;

	const incompatible = isVersionIncompatible(
		template.min_ironflow_version,
		ironflowVersion,
	);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent>
				<SheetHeader>
					<SheetTitle>{template.name}</SheetTitle>
					<SheetDescription>{template.description}</SheetDescription>
				</SheetHeader>
				<div className="space-y-6 p-6">
					<div className="space-y-4">
						{template.latest_version && (
							<div className="flex items-center gap-2 text-sm">
								<Tag className="h-4 w-4 text-muted-foreground" />
								<span className="text-muted-foreground">Version:</span>
								<span className="font-medium">{template.latest_version}</span>
							</div>
						)}

						{template.authors && template.authors.length > 0 && (
							<div className="flex items-center gap-2 text-sm">
								<User className="h-4 w-4 text-muted-foreground" />
								<span className="text-muted-foreground">
									{template.authors.length === 1 ? "Author:" : "Authors:"}
								</span>
								<span className="font-medium">
									{template.authors.join(", ")}
								</span>
							</div>
						)}

						{template.category && (
							<div className="flex items-center gap-2 text-sm">
								<Package className="h-4 w-4 text-muted-foreground" />
								<span className="text-muted-foreground">Category:</span>
								<Badge variant="secondary">{template.category}</Badge>
							</div>
						)}

						{template.min_ironflow_version && (
							<div className="flex items-center gap-2 text-sm">
								{incompatible ? (
									<XCircle className="h-4 w-4 text-destructive" />
								) : (
									<Check className="h-4 w-4 text-green-500" />
								)}
								<span className="text-muted-foreground">
									Requires Ironflow:
								</span>
								<span
									className={
										incompatible
											? "font-medium text-destructive"
											: "font-medium text-green-600 dark:text-green-400"
									}
								>
									{">="} {template.min_ironflow_version}
								</span>
								{incompatible && (
									<span className="text-xs text-destructive">
										(current: {ironflowVersion})
									</span>
								)}
							</div>
						)}

						<div className="flex items-center gap-2 text-sm">
							<ExternalLink className="h-4 w-4 text-muted-foreground" />
							<span className="text-muted-foreground">Repository:</span>
							<a
								href={template.repo}
								target="_blank"
								rel="noopener noreferrer"
								className="font-medium text-primary hover:underline truncate"
							>
								{template.repo}
							</a>
						</div>
					</div>

					<div className="space-y-2">
						<p className="text-sm font-medium">Install</p>
						<CopyButton text={template.name} />
					</div>
				</div>
			</SheetContent>
		</Sheet>
	);
}

export function Component() {
	useDocumentMeta({ title: "Templates" });
	const { templates, ironflow_version, error } = useLoaderData() as LoaderData;

	const [search, setSearch] = useQueryState(
		"q",
		parseAsString
			.withDefault("")
			.withOptions({ shallow: false, throttleMs: 300 }),
	);

	const [categoryFilters, setCategoryFilters] = useQueryStates(
		{
			categories: parseAsArrayOf(parseAsString).withDefault([]),
		},
		{ shallow: false },
	);

	const categories = [
		...new Set(templates.map((t) => t.category).filter(Boolean)),
	] as string[];

	const categoryOptions = categories.map((cat) => ({
		value: cat,
		label: cat,
	}));

	const filtered = templates.filter((t) => {
		const matchesSearch =
			!search ||
			t.name.toLowerCase().includes(search.toLowerCase()) ||
			t.description.toLowerCase().includes(search.toLowerCase());
		const matchesCategory =
			categoryFilters.categories.length === 0 ||
			(t.category && categoryFilters.categories.includes(t.category));
		return matchesSearch && matchesCategory;
	});

	const [selectedTemplate, setSelectedTemplate] =
		useState<TemplateEntry | null>(null);
	const [sheetOpen, setSheetOpen] = useState(false);

	return (
		<>
			<HeaderApp
				title="Templates"
				description="Browse available workflow templates from the registry"
			/>
			<div className="p-4 sm:p-6 space-y-6 overflow-x-hidden">
				{error && (
					<div className="flex items-center gap-3 rounded-md border border-yellow-300 bg-yellow-50 dark:border-yellow-700 dark:bg-yellow-950 p-4 text-sm text-yellow-800 dark:text-yellow-200">
						<AlertTriangle className="h-5 w-5 shrink-0" />
						<p>{error}</p>
					</div>
				)}
				<div className="flex flex-col sm:flex-row gap-3">
					<Input
						placeholder="Search templates..."
						value={search}
						onChange={(e) => setSearch(e.target.value)}
						className="w-full sm:w-64"
					/>
					{categories.length > 0 && (
						<MultiSelect
							options={categoryOptions}
							value={categoryFilters.categories}
							onChange={(v) => setCategoryFilters({ categories: v })}
							placeholder="All categories"
							className="w-full sm:w-64"
						/>
					)}
				</div>

				{filtered.length === 0 ? (
					<div className="text-center py-12 text-muted-foreground">
						<Package className="h-12 w-12 mx-auto mb-3 opacity-50" />
						<p>No templates found</p>
					</div>
				) : (
					<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
						{filtered.map((template) => {
							const incompatible = isVersionIncompatible(
								template.min_ironflow_version,
								ironflow_version,
							);

							return (
								<Card
									key={template.name}
									className="min-w-0 overflow-hidden cursor-pointer hover:border-primary/50 transition-colors"
									onClick={() => {
										setSelectedTemplate(template);
										setSheetOpen(true);
									}}
								>
									<CardHeader className="pb-3">
										<div className="flex items-start justify-between gap-2">
											<div className="space-y-1 min-w-0">
												<CardTitle className="text-base">
													{template.name}
												</CardTitle>
												{template.latest_version && (
													<span className="text-xs text-muted-foreground">
														v{template.latest_version}
													</span>
												)}
											</div>
											{template.category && (
												<Badge variant="secondary" className="text-xs shrink-0">
													{template.category}
												</Badge>
											)}
										</div>
										<CardDescription>{template.description}</CardDescription>
									</CardHeader>
									<CardContent className="flex flex-1 flex-col">
										{template.authors && template.authors.length > 0 && (
											<p className="text-xs text-muted-foreground">
												by {template.authors.join(", ")}
											</p>
										)}
										<div className="mt-auto space-y-2 pt-3">
											{incompatible && (
												<div className="flex items-center gap-1.5 text-xs text-destructive">
													<XCircle className="h-3.5 w-3.5 shrink-0" />
													Requires Ironflow{" >= "}
													{template.min_ironflow_version}
												</div>
											)}
											<CopyButton text={template.name} />
										</div>
									</CardContent>
								</Card>
							);
						})}
					</div>
				)}
			</div>

			<TemplateDetailSheet
				template={selectedTemplate}
				ironflowVersion={ironflow_version}
				open={sheetOpen}
				onOpenChange={setSheetOpen}
			/>
		</>
	);
}
