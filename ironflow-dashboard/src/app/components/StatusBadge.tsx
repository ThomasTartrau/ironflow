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
				return "bg-[var(--status-pending-bg)] text-[var(--status-pending-fg)] border-[var(--status-pending-border)]";
			case "running":
				return "bg-[var(--status-running-bg)] text-[var(--status-running-fg)] border-[var(--status-running-border)] animate-pulse";
			case "completed":
				return "bg-[var(--status-completed-bg)] text-[var(--status-completed-fg)] border-[var(--status-completed-border)]";
			case "failed":
				return "bg-[var(--status-failed-bg)] text-[var(--status-failed-fg)] border-[var(--status-failed-border)]";
			case "retrying":
				return "bg-[var(--status-retrying-bg)] text-[var(--status-retrying-fg)] border-[var(--status-retrying-border)]";
			case "awaiting_approval":
				return "bg-[var(--status-awaiting-bg)] text-[var(--status-awaiting-fg)] border-[var(--status-awaiting-border)] animate-pulse";
			case "rejected":
				return "bg-[var(--status-rejected-bg)] text-[var(--status-rejected-fg)] border-[var(--status-rejected-border)]";
			case "cancelled":
			case "skipped":
				return "bg-[var(--status-cancelled-bg)] text-[var(--status-cancelled-fg)] border-[var(--status-cancelled-border)]";
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
			className={`text-[11px] font-medium uppercase tracking-wide tabular-nums ${getStatusStyles(status)}`}
		>
			{displayLabel}
		</Badge>
	);
}
