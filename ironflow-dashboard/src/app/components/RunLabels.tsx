import { Badge } from "@/components/ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";

const MAX_VISIBLE = 2;

interface RunLabelsProps {
	labels?: Record<string, string>;
}

export function RunLabels({ labels }: RunLabelsProps) {
	if (!labels) return null;
	const entries = Object.entries(labels);
	if (entries.length === 0) return null;

	const visible = entries.slice(0, MAX_VISIBLE);
	const remaining = entries.length - MAX_VISIBLE;

	return (
		<div className="flex items-center gap-1 flex-wrap">
			{visible.map(([key, value]) => (
				<Badge
					key={key}
					variant="secondary"
					className="font-mono text-[10px] px-1.5 py-0"
				>
					{key}: {value}
				</Badge>
			))}
			{remaining > 0 && (
				<TooltipProvider>
					<Tooltip>
						<TooltipTrigger>
							<Badge
								variant="outline"
								className="text-[10px] px-1.5 py-0 cursor-default"
							>
								+{remaining}
							</Badge>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="max-w-xs">
							<div className="flex flex-col gap-1">
								{entries.slice(MAX_VISIBLE).map(([key, value]) => (
									<span key={key} className="font-mono text-xs">
										{key}: {value}
									</span>
								))}
							</div>
						</TooltipContent>
					</Tooltip>
				</TooltipProvider>
			)}
		</div>
	);
}
