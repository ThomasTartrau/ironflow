import { useState } from "react";
import { useNavigate, useRevalidator } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { withToast, type ToastMessages } from "@/app/lib/api-toast";
import { cancelRun, retryRun } from "../_actions/actions";
import { Button } from "@/components/ui/button";

interface RunActionsProps {
	run: RunResponse;
}

type PendingAction = "idle" | "cancelling" | "retrying";

export function RunActions({ run }: RunActionsProps) {
	const revalidator = useRevalidator();
	const navigate = useNavigate();
	const [pendingAction, setPendingAction] = useState<PendingAction>("idle");

	const canCancel = run.status === "pending" || run.status === "running";
	const canRetry = run.status === "failed" || run.status === "cancelled";
	const isLoading = pendingAction !== "idle";

	const handleAction = (
		action: PendingAction,
		fn: () => Promise<unknown>,
		messages: ToastMessages,
		onSuccess?: (result: unknown) => void,
	) => {
		setPendingAction(action);
		withToast(fn(), messages)
			.then((result) => {
				if (onSuccess) {
					onSuccess(result);
				} else {
					revalidator.revalidate();
				}
			})
			.catch(() => {})
			.finally(() => setPendingAction("idle"));
	};

	return (
		<div className="flex gap-2">
			{canCancel && (
				<Button
					onClick={() =>
						handleAction("cancelling", () => cancelRun(run.id), {
							loading: "Cancelling run...",
							success: "Run cancelled",
							error: "Failed to cancel run",
						})
					}
					disabled={isLoading}
					variant="destructive"
				>
					{pendingAction === "cancelling" ? "Cancelling..." : "Cancel"}
				</Button>
			)}
			{canRetry && (
				<Button
					onClick={() =>
						handleAction(
							"retrying",
							() => retryRun(run.id),
							{
								loading: "Retrying run...",
								success: "Run queued for retry",
								error: "Failed to retry run",
							},
							(result) => {
								const newRun = result as RunResponse;
								navigate(`/runs/${newRun.id}`);
							},
						)
					}
					disabled={isLoading}
					variant="outline"
				>
					{pendingAction === "retrying" ? "Retrying..." : "Retry"}
				</Button>
			)}
		</div>
	);
}
