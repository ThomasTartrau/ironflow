import { useState } from "react";
import {
	useQueryStates,
	parseAsString,
	parseAsBoolean,
	parseAsArrayOf,
	debounce,
} from "nuqs";
import { Filter, X, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
	Sheet,
	SheetContent,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
} from "@/components/ui/sheet";
import { Badge } from "@/components/ui/badge";
import type { RunStatus } from "@/app/lib/types";
import { capitalize } from "@/app/lib/format";

const STATUS_OPTIONS: RunStatus[] = [
	"pending",
	"running",
	"completed",
	"failed",
	"retrying",
	"cancelled",
	"awaiting_approval",
];

export function RunFilters() {
	const [open, setOpen] = useState(false);
	const [labelInput, setLabelInput] = useState("");
	const [filters, setFilters] = useQueryStates(
		{
			workflow: parseAsString.withDefault("").withOptions({
				shallow: false,
				limitUrlUpdates: debounce(300),
			}),
			status: parseAsString.withDefault("").withOptions({
				shallow: false,
			}),
			has_steps: parseAsBoolean.withDefault(true).withOptions({
				shallow: false,
			}),
			label: parseAsArrayOf(parseAsString).withDefault([]).withOptions({
				shallow: false,
			}),
			page: parseAsString.withDefault("1").withOptions({
				shallow: false,
			}),
		},
		{ history: "replace" },
	);

	const activeCount = [
		filters.workflow,
		filters.status,
		!filters.has_steps ? "has_steps" : "",
		filters.label.length > 0 ? "label" : "",
	].filter(Boolean).length;

	const handleWorkflowChange = (value: string) => {
		setFilters({ workflow: value || null, page: "1" });
	};

	const handleStatusChange = (value: string | null) => {
		setFilters({ status: value || null, page: "1" });
	};

	const handleHasStepsChange = (checked: boolean) => {
		setFilters({ has_steps: checked, page: "1" });
	};

	const handleAddLabel = () => {
		const trimmed = labelInput.trim();
		const colonIdx = trimmed.indexOf(":");
		if (colonIdx < 1) return;
		const key = trimmed.slice(0, colonIdx).trim();
		const value = trimmed.slice(colonIdx + 1).trim();
		if (!key || !value) return;
		const normalized = `${key}:${value}`;
		if (filters.label.includes(normalized)) return;
		setFilters({ label: [...filters.label, normalized], page: "1" });
		setLabelInput("");
	};

	const handleRemoveLabel = (label: string) => {
		const next = filters.label.filter((l) => l !== label);
		setFilters({ label: next.length > 0 ? next : null, page: "1" });
	};

	const handleLabelKeyDown = (e: React.KeyboardEvent) => {
		if (e.key === "Enter") {
			e.preventDefault();
			handleAddLabel();
		}
	};

	const handleReset = () => {
		setFilters({
			workflow: null,
			status: null,
			has_steps: null,
			label: null,
			page: null,
		});
		setLabelInput("");
	};

	return (
		<div className="flex flex-wrap items-center gap-2">
			<Sheet open={open} onOpenChange={setOpen}>
				<SheetTrigger
					render={
						<Button variant="outline" className="gap-1.5">
							<Filter className="h-4 w-4" />
							Filters
							{activeCount > 0 && (
								<Badge variant="secondary" className="ml-1 px-1.5 py-0 text-xs">
									{activeCount}
								</Badge>
							)}
						</Button>
					}
				/>
				<SheetContent>
					<SheetHeader>
						<SheetTitle>Filters</SheetTitle>
					</SheetHeader>
					<div className="space-y-6 px-4 py-6">
						<div>
							<label htmlFor="filter-workflow" className="text-sm font-medium">
								Workflow Name
							</label>
							<Input
								id="filter-workflow"
								placeholder="Filter by workflow name..."
								value={filters.workflow}
								onChange={(e) => handleWorkflowChange(e.target.value)}
								className="mt-1.5 font-mono"
							/>
						</div>

						<div>
							<label htmlFor="filter-status" className="text-sm font-medium">
								Status
							</label>
							<Select value={filters.status} onValueChange={handleStatusChange}>
								<SelectTrigger className="mt-1.5">
									<SelectValue placeholder="All statuses" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="">All statuses</SelectItem>
									{STATUS_OPTIONS.map((s) => (
										<SelectItem key={s} value={s}>
											{capitalize(s)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>

						<div>
							<label htmlFor="filter-label" className="text-sm font-medium">
								Labels
							</label>
							<div className="flex gap-1.5 mt-1.5">
								<Input
									id="filter-label"
									placeholder="key:value"
									value={labelInput}
									onChange={(e) => setLabelInput(e.target.value)}
									onKeyDown={handleLabelKeyDown}
									className="font-mono"
								/>
								<Button
									variant="outline"
									size="icon"
									onClick={handleAddLabel}
									disabled={!labelInput.includes(":")}
								>
									<Plus className="h-4 w-4" />
								</Button>
							</div>
							{filters.label.length > 0 && (
								<div className="flex flex-wrap gap-1.5 mt-2">
									{filters.label.map((l) => (
										<button
											key={l}
											type="button"
											aria-label={`Remove label filter ${l}`}
											onClick={() => handleRemoveLabel(l)}
											onKeyDown={(e) => {
												if (e.key === "Enter" || e.key === " ") {
													e.preventDefault();
													handleRemoveLabel(l);
												}
											}}
											className="inline-flex items-center gap-1 font-mono text-xs px-1.5 py-0.5 rounded-[var(--radius-sm)] bg-secondary text-secondary-foreground hover:bg-secondary/80 cursor-pointer"
										>
											{l}
											<X className="h-3 w-3" aria-hidden="true" />
										</button>
									))}
								</div>
							)}
						</div>

						<div className="flex items-center justify-between">
							<label
								htmlFor="filter-has-steps"
								className="text-sm font-medium cursor-pointer select-none"
							>
								Hide empty runs
							</label>
							<Switch
								id="filter-has-steps"
								checked={filters.has_steps}
								onCheckedChange={handleHasStepsChange}
							/>
						</div>

						<Button onClick={handleReset} variant="outline" className="w-full">
							<X className="h-4 w-4 mr-1.5" />
							Remove all filters
						</Button>
					</div>
				</SheetContent>
			</Sheet>

			{filters.workflow && (
				<button
					type="button"
					aria-label="Remove workflow filter"
					onClick={() => setFilters({ workflow: null, page: "1" })}
					className="inline-flex items-center gap-1 font-mono text-xs px-1.5 py-0.5 rounded-[var(--radius-sm)] bg-secondary text-secondary-foreground hover:bg-secondary/80 cursor-pointer"
				>
					workflow: {filters.workflow}
					<X className="h-3 w-3" aria-hidden="true" />
				</button>
			)}
			{filters.status && (
				<button
					type="button"
					aria-label="Remove status filter"
					onClick={() => setFilters({ status: null, page: "1" })}
					className="inline-flex items-center gap-1 font-mono text-xs px-1.5 py-0.5 rounded-[var(--radius-sm)] bg-secondary text-secondary-foreground hover:bg-secondary/80 cursor-pointer"
				>
					status: {filters.status}
					<X className="h-3 w-3" aria-hidden="true" />
				</button>
			)}
			{filters.label.map((l) => (
				<button
					key={l}
					type="button"
					aria-label={`Remove label filter ${l}`}
					onClick={() => handleRemoveLabel(l)}
					className="inline-flex items-center gap-1 font-mono text-xs px-1.5 py-0.5 rounded-[var(--radius-sm)] bg-secondary text-secondary-foreground hover:bg-secondary/80 cursor-pointer"
				>
					{l}
					<X className="h-3 w-3" aria-hidden="true" />
				</button>
			))}

			{activeCount > 0 && (
				<Button
					variant="ghost"
					size="sm"
					onClick={handleReset}
					className="text-muted-foreground"
				>
					<X className="h-3.5 w-3.5 mr-1" />
					Clear all
				</Button>
			)}
		</div>
	);
}
