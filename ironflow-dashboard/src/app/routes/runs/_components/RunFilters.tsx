import { useQueryStates, parseAsString, debounce } from "nuqs";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
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
	const [filters, setFilters] = useQueryStates(
		{
			workflow: parseAsString.withDefault("").withOptions({
				shallow: false,
				limitUrlUpdates: debounce(300),
			}),
			status: parseAsString.withDefault("").withOptions({
				shallow: false,
			}),
			page: parseAsString.withDefault("1").withOptions({
				shallow: false,
			}),
		},
		{ history: "replace" },
	);

	const handleWorkflowChange = (value: string) => {
		setFilters({ workflow: value || null, page: "1" });
	};

	const handleStatusChange = (value: string | null) => {
		setFilters({ status: value || null, page: "1" });
	};

	const handleReset = () => {
		setFilters({ workflow: null, status: null, page: null });
	};

	return (
		<div className="mt-6 flex flex-col gap-4 md:flex-row md:items-end">
			<div className="flex-1">
				<label htmlFor="filter-workflow" className="text-sm font-medium">
					Workflow Name
				</label>
				<Input
					id="filter-workflow"
					placeholder="Filter by workflow name..."
					value={filters.workflow}
					onChange={(e) => handleWorkflowChange(e.target.value)}
					className="mt-1"
				/>
			</div>

			<div className="w-full md:w-48">
				<label htmlFor="filter-status" className="text-sm font-medium">
					Status
				</label>
				<Select value={filters.status} onValueChange={handleStatusChange}>
					<SelectTrigger className="mt-1">
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

			<Button onClick={handleReset} variant="outline">
				Remove all filters
			</Button>
		</div>
	);
}
