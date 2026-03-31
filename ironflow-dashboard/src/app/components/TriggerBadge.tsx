import type { TriggerKind } from "@/app/lib/types";
import { Badge } from "@/components/ui/badge";

interface TriggerBadgeProps {
	trigger: TriggerKind;
}

export function TriggerBadge({ trigger }: TriggerBadgeProps) {
	const getLabel = (t: TriggerKind): string => {
		switch (t.kind) {
			case "manual":
				return "Manual";
			case "webhook":
				return `Webhook: ${t.path}`;
			case "cron":
				return `Cron: ${t.schedule}`;
			case "api":
				return "API";
			case "retry":
				return `Retry: ${t.parent_run_id}`;
			case "workflow":
				return "Workflow";
			default: {
				const _exhaustive: never = t;
				return _exhaustive;
			}
		}
	};

	return <Badge variant="outline">{getLabel(trigger)}</Badge>;
}
