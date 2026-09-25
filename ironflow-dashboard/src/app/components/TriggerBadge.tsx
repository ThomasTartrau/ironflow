import { Link } from "react-router";
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
	href?: string | null;
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
		case "replay":
			return {
				label: "Replay",
				tooltip: t.original_run_id,
				href: `/runs/${t.original_run_id}`,
			};
		case "workflow":
			return { label: "Workflow", tooltip: null };
		case "nats":
			return { label: "NATS", tooltip: t.subject };
		case "run_event":
			return {
				label: "Event",
				tooltip: `${t.event_kind} (${t.source_run_id})`,
			};
		case "polling":
			return { label: "Polling", tooltip: t.probe };
		default: {
			const _exhaustive: never = t;
			return _exhaustive;
		}
	}
}

export function TriggerBadge({ trigger }: TriggerBadgeProps) {
	const { label, tooltip, href } = getTriggerMeta(trigger);

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
					{href ? (
						<Link
							to={href}
							// The badge sits inside clickable table rows: keep the click
							// from also triggering the row navigation.
							onClick={(e) => e.stopPropagation()}
							className="font-mono text-xs underline"
						>
							{tooltip}
						</Link>
					) : (
						<span className="font-mono text-xs">{tooltip}</span>
					)}
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
}
