import { RotateCcw } from "lucide-react";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { attemptLabel } from "./attempts";

type AttemptSelectorProps = {
	attempts: number[];
	value: number;
	latest: number;
	onChange: (attempt: number) => void;
};

/**
 * Picks which attempt of a retried run the step views show.
 *
 * Steps of every attempt come back on the same run; each view renders a single
 * attempt so the timeline and the DAG stay coherent.
 */
export function AttemptSelector({
	attempts,
	value,
	latest,
	onChange,
}: AttemptSelectorProps) {
	return (
		<div className="flex items-center gap-2">
			<RotateCcw className="size-4 text-muted-foreground" />
			<span className="text-sm text-muted-foreground">Attempt</span>
			<Select
				value={String(value)}
				onValueChange={(next) => onChange(Number(next))}
			>
				<SelectTrigger className="w-[180px]" size="sm">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					{attempts.map((attempt) => (
						<SelectItem key={attempt} value={String(attempt)}>
							{attemptLabel(attempt, latest)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
		</div>
	);
}
