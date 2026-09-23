import type {
	ExecutionPlanResponse,
	PlannedStepResponse,
} from "@/app/lib/types";
import { formatDuration } from "@/app/lib/format";
import { getKindMeta, nodeColors } from "@/app/lib/step-kind";
import { AlertTriangle } from "lucide-react";

/** A run of consecutive steps that render as one row of the plan. */
type PlanRow =
	| { kind: "step"; step: PlannedStepResponse; index: number }
	| {
			kind: "parallel";
			group: string;
			members: { step: PlannedStepResponse; index: number }[];
	  };

/**
 * Group consecutive steps sharing a `parallel_group` into a single row, so the
 * plan reads top to bottom the way it executes.
 */
function toRows(steps: PlannedStepResponse[]): PlanRow[] {
	const rows: PlanRow[] = [];
	for (let index = 0; index < steps.length; index += 1) {
		const step = steps[index];
		const group = step.parallel_group;
		if (!group) {
			rows.push({ kind: "step", step, index });
			continue;
		}
		const last = rows[rows.length - 1];
		if (last && last.kind === "parallel" && last.group === group) {
			last.members.push({ step, index });
		} else {
			rows.push({ kind: "parallel", group, members: [{ step, index }] });
		}
	}
	return rows;
}

function conditionBadge(step: PlannedStepResponse): string | null {
	const condition = step.condition;
	if (!condition) return null;
	switch (condition.state) {
		case "evaluated":
			return `when ${condition.expression ?? "?"} = ${condition.value ? "true" : "false"}`;
		case "skipped":
			return `skipped: ${condition.reason ?? "no reason given"}`;
		default:
			return `unevaluable: ${condition.expression ?? "?"}`;
	}
}

function PlanNode({ step }: { step: PlannedStepResponse }) {
	const meta = getKindMeta(step.kind);
	const Icon = meta.icon;
	const badge = conditionBadge(step);

	return (
		<div
			data-testid="plan-node"
			className={`flex-1 rounded-[var(--radius)] border ${nodeColors[meta.color]} bg-card px-2 py-1.5`}
		>
			<div className="flex items-center gap-2">
				<span
					className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-[var(--radius-sm)] border ${nodeColors[meta.color]} bg-card`}
				>
					<Icon className="size-2.5" aria-hidden="true" />
				</span>
				<span className="truncate font-mono text-xs">{step.name}</span>
				<span className="ml-auto text-[10px] text-muted-foreground">
					{meta.label}
				</span>
				{step.estimated_duration_ms != null && (
					<span className="text-[10px] text-muted-foreground">
						~{formatDuration(step.estimated_duration_ms)}
					</span>
				)}
			</div>
			{badge && (
				<p className="mt-1 text-[10px] text-muted-foreground">{badge}</p>
			)}
		</div>
	);
}

interface ExecutionPlanGraphProps {
	plan: ExecutionPlanResponse;
}

/**
 * Render an execution plan as a top-to-bottom graph: one row per sequential
 * step, one bordered box per parallel wave, and an indented box per
 * sub-workflow level.
 */
export function ExecutionPlanGraph({ plan }: ExecutionPlanGraphProps) {
	const rows = toRows(plan.steps);

	return (
		<div className="space-y-2" data-testid="execution-plan">
			<div className="flex items-baseline gap-2 text-xs text-muted-foreground">
				<span className="font-mono">{plan.workflow}</span>
				{plan.estimated_duration_ms != null && (
					<span>~{formatDuration(plan.estimated_duration_ms)}</span>
				)}
				<span className="ml-auto">
					{plan.steps.length} step{plan.steps.length === 1 ? "" : "s"}
				</span>
			</div>

			{plan.steps.length === 0 && (
				<p className="text-xs text-muted-foreground">
					This workflow plans no step for the current input.
				</p>
			)}

			{rows.map((row) =>
				row.kind === "step" ? (
					<div
						key={`step-${row.index}`}
						className="flex"
						style={{ paddingLeft: `${row.step.depth * 16}px` }}
					>
						{row.step.depth > 0 ? (
							<div className="flex-1 rounded-[var(--radius)] border border-border/60 border-dashed p-1.5">
								<p className="mb-1 font-mono text-[10px] text-muted-foreground">
									{row.step.workflow}
								</p>
								<PlanNode step={row.step} />
							</div>
						) : (
							<PlanNode step={row.step} />
						)}
					</div>
				) : (
					<div
						key={`group-${row.group}`}
						data-testid="parallel-group"
						className="rounded-[var(--radius)] border border-border/60 border-dashed p-1.5"
						style={{ marginLeft: `${row.members[0].step.depth * 16}px` }}
					>
						<p className="mb-1 font-mono text-[10px] text-muted-foreground">
							{row.group}
						</p>
						<div className="flex flex-col gap-1.5 sm:flex-row">
							{row.members.map((member) => (
								<PlanNode key={`member-${member.index}`} step={member.step} />
							))}
						</div>
					</div>
				),
			)}

			{plan.truncated && (
				<p
					role="status"
					className="flex items-center gap-1.5 rounded-[var(--radius)] border border-amber-400/40 bg-amber-400/10 px-2 py-1.5 text-[11px] text-amber-600 dark:text-amber-300"
				>
					<AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
					Plan incomplete: {plan.incomplete_reason ?? "the plan was cut short"}
				</p>
			)}
		</div>
	);
}
