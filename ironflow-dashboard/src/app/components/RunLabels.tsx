import { Badge } from "@/components/ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";

const DEFAULT_MAX_VISIBLE = 2;

interface RunLabelsProps {
	labels?: Record<string, string>;
	maxVisible?: number;
}

export function RunLabels({
	labels,
	maxVisible = DEFAULT_MAX_VISIBLE,
}: RunLabelsProps) {
	if (!labels) return null;
	const entries = Object.entries(labels);
	if (entries.length === 0) return null;

	const visible = entries.slice(0, maxVisible);
	const remaining = entries.length - maxVisible;

	return (
		<TooltipProvider delay={200}>
			<div className="flex items-center gap-1 flex-wrap min-w-0 max-w-full">
				{visible.map(([key, value]) => (
					<Tooltip key={key}>
						<TooltipTrigger
							render={
								<Badge
									variant="secondary"
									className="font-mono text-[10px] px-1.5 py-0 max-w-full min-w-0 shrink justify-start cursor-default"
								>
									<span className="truncate">
										{key}: {value}
									</span>
								</Badge>
							}
						/>
						<TooltipContent side="bottom" className="max-w-xs break-all">
							<span className="font-mono text-xs">
								{key}: {value}
							</span>
						</TooltipContent>
					</Tooltip>
				))}
				{remaining > 0 && (
					<Tooltip>
						<TooltipTrigger
							render={
								<Badge
									variant="outline"
									className="text-[10px] px-1.5 py-0 cursor-default shrink-0"
								>
									+{remaining}
								</Badge>
							}
						/>
						<TooltipContent side="bottom" className="max-w-xs">
							<div className="flex flex-col gap-1">
								{entries.slice(maxVisible).map(([key, value]) => (
									<span key={key} className="font-mono text-xs">
										{key}: {value}
									</span>
								))}
							</div>
						</TooltipContent>
					</Tooltip>
				)}
			</div>
		</TooltipProvider>
	);
}
