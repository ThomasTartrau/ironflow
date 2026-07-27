import { Bot, KeyRound, User } from "lucide-react";
import type { CreatedBy, CreatedByKind } from "@/app/lib/types";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";

interface CreatedByBadgeProps {
	createdBy: CreatedBy;
}

const ICONS = {
	user: User,
	api_key: KeyRound,
	system: Bot,
} as const satisfies Record<CreatedByKind, unknown>;

const TOOLTIPS = {
	user: "Triggered by a user",
	api_key: "Triggered by an API key",
	system: "Triggered automatically",
} as const satisfies Record<CreatedByKind, string>;

export function CreatedByBadge({ createdBy }: CreatedByBadgeProps) {
	const Icon = ICONS[createdBy.kind];

	return (
		<TooltipProvider delay={200}>
			<Tooltip>
				<TooltipTrigger
					render={
						<span className="inline-flex items-center gap-1 text-xs text-muted-foreground min-w-0">
							<Icon className="size-3 shrink-0" aria-hidden="true" />
							<span className="font-mono truncate">{createdBy.label}</span>
						</span>
					}
				/>
				<TooltipContent side="bottom">
					<span className="text-xs">
						{TOOLTIPS[createdBy.kind]}: {createdBy.label}
					</span>
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
}
