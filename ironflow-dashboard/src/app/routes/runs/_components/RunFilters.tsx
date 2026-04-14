import { useState } from "react";
import { useQueryStates, parseAsString, parseAsBoolean, debounce } from "nuqs";
import { Filter, X } from "lucide-react";
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

	const handleReset = () => {
		setFilters({ workflow: null, status: null, has_steps: null, page: null });
	};

	return (
		<div className="flex items-center gap-2">
			<Sheet open={open} onOpenChange={setOpen}>
				<SheetTrigger>
					<Button variant="outline" className="gap-1.5">
						<Filter className="h-4 w-4" />
						Filters
						{activeCount > 0 && (
							<Badge variant="secondary" className="ml-1 px-1.5 py-0 text-xs">
								{activeCount}
							</Badge>
						)}
					</Button>
				</SheetTrigger>
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
								className="mt-1.5"
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

			{activeCount > 0 && (
				<Button
					variant="ghost"
					size="sm"
					onClick={handleReset}
					className="text-muted-foreground"
				>
					<X className="h-3.5 w-3.5 mr-1" />
					Clear
				</Button>
			)}
		</div>
	);
}
