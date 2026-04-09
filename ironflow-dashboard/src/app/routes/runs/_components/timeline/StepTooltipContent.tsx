import { Badge } from "@/components/ui/badge";
import { formatDuration, formatCost } from "@/app/lib/format";
import type { TimelineRow } from "./types";
import { getKindMeta } from "./types";

interface StepTooltipContentProps {
	row: TimelineRow;
}

export function StepTooltipContent({ row }: StepTooltipContentProps) {
	const meta = getKindMeta(row.step.kind);
	const Icon = meta.icon;
	return (
		<div className="space-y-1.5 text-left min-w-[180px]">
			<div className="flex items-center gap-1.5">
				<Icon className={`h-3.5 w-3.5 ${meta.color}`} />
				<span className="font-semibold text-[13px]">{row.step.name}</span>
			</div>
			<div className="flex items-center gap-2 text-[11px]">
				<Badge
					variant="outline"
					className={`${meta.badgeCls} text-[10px] px-1.5 py-0`}
				>
					{meta.label}
				</Badge>
				<span className="capitalize">{row.step.status}</span>
			</div>
			<div className="grid grid-cols-2 gap-x-3 gap-y-0.5 text-[11px] pt-0.5">
				<span className="text-background/60">Duration</span>
				<span className="font-medium">
					{formatDuration(row.step.duration_ms)}
				</span>
				{row.step.cost_usd > 0 && (
					<>
						<span className="text-background/60">Cost</span>
						<span className="font-medium">{formatCost(row.step.cost_usd)}</span>
					</>
				)}
				<span className="text-background/60">Offset</span>
				<span className="font-medium">+{formatDuration(row.startMs)}</span>
			</div>
		</div>
	);
}
