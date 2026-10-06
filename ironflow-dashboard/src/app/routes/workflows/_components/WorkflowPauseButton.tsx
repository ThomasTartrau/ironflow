import { useState } from "react";
import { useRevalidator } from "react-router";
import { withToast } from "@/app/lib/api-toast";
import { Button } from "@/components/ui/button";
import { pauseWorkflow, resumeWorkflow } from "../_actions/actions";

interface WorkflowPauseButtonProps {
	name: string;
	/** When the workflow was paused, absent when it is not paused. */
	pausedAt?: string | null;
}

/**
 * Pause or resume a whole workflow. A paused workflow keeps receiving runs,
 * but workers leave them queued until it is resumed.
 */
export function WorkflowPauseButton({
	name,
	pausedAt,
}: WorkflowPauseButtonProps) {
	const revalidator = useRevalidator();
	const [pending, setPending] = useState(false);
	const paused = Boolean(pausedAt);

	const toggle = () => {
		setPending(true);
		const request = paused
			? withToast(resumeWorkflow(name), {
					loading: "Resuming workflow...",
					success: "Workflow resumed",
					error: "Failed to resume workflow",
				})
			: withToast(pauseWorkflow(name), {
					loading: "Pausing workflow...",
					success: "Workflow paused: queued runs wait until it is resumed",
					error: "Failed to pause workflow",
				});
		request
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setPending(false));
	};

	if (paused) {
		return (
			<Button onClick={toggle} disabled={pending} variant="default">
				{pending ? "Resuming..." : "Resume workflow"}
			</Button>
		);
	}
	return (
		<Button onClick={toggle} disabled={pending} variant="outline">
			{pending ? "Pausing..." : "Pause workflow"}
		</Button>
	);
}
