import type { TriggerKind } from "@/app/lib/types";
import { Badge } from "@/components/ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";

interface TriggerBadgeProps {
	trigger: TriggerKind;
}

interface TriggerMeta {
	label: string;
	tooltip: string | null;
}

function getTriggerMeta(t: TriggerKind): TriggerMeta {
	switch (t.kind) {
		case "manual":
			return { label: "Manual", tooltip: null };
		case "webhook":
			return { label: "Webhook", tooltip: t.path };
		case "cron":
			return { label: "Cron", tooltip: t.schedule };
		case "api":
			return { label: "API", tooltip: null };
		case "retry":
			return { label: "Retry", tooltip: t.parent_run_id };
		case "workflow":
			return { label: "Workflow", tooltip: null };
		case "nats":
			return { label: "NATS", tooltip: t.subject };
		case "run_event":
			return { label: "Event", tooltip: `${t.event_kind} (${t.source_run_id})` };
		default: {
			const _exhaustive: never = t;
			return _exhaustive;
		}
	}
}

export function TriggerBadge({ trigger }: TriggerBadgeProps) {
	const { label, tooltip } = getTriggerMeta(trigger);

	if (!tooltip) {
		return (
			<Badge variant="outline" className="font-mono text-[10px]">
				{label}
			</Badge>
		);
	}

	return (
		<TooltipProvider delay={200}>
			<Tooltip>
				<TooltipTrigger
					render={
						<Badge variant="outline" className="font-mono text-[10px]">
							{label}
						</Badge>
					}
				/>
				<TooltipContent side="bottom">
					<span className="font-mono text-xs">{tooltip}</span>
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
}
