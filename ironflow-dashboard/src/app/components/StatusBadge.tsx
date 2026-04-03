import type { RunStatus, StepStatus } from "@/app/lib/types";
import { capitalize } from "@/app/lib/format";
import { Badge } from "@/components/ui/badge";

type Status = RunStatus | StepStatus;

interface StatusBadgeProps {
	status: Status;
}

export function StatusBadge({ status }: StatusBadgeProps) {
	const getStatusStyles = (stat: Status): string => {
		switch (stat) {
			case "pending":
				return "bg-amber-100 text-amber-700 border-amber-200";
			case "running":
				return "bg-blue-100 text-blue-700 border-blue-200 animate-pulse";
			case "completed":
				return "bg-emerald-100 text-emerald-700 border-emerald-200";
			case "failed":
				return "bg-red-100 text-red-700 border-red-200";
			case "retrying":
				return "bg-orange-100 text-orange-700 border-orange-200";
			case "awaiting_approval":
				return "bg-purple-100 text-purple-700 border-purple-200 animate-pulse";
			case "cancelled":
			case "skipped":
				return "bg-gray-100 text-gray-600 border-gray-200";
			default: {
				const _exhaustive: never = stat;
				return _exhaustive;
			}
		}
	};

	const displayLabel = capitalize(status);

	return (
		<Badge
			variant="outline"
			className={`text-xs font-medium ${getStatusStyles(status)}`}
		>
			{displayLabel}
		</Badge>
	);
}
