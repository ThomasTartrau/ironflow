import { useState } from "react";
import { useNavigate, useRevalidator } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { withToast, type ToastMessages } from "@/app/lib/api-toast";
import { approveRun, cancelRun, rejectRun, retryRun } from "../_actions/actions";
import { Button } from "@/components/ui/button";

interface RunActionsProps {
	run: RunResponse;
}

type PendingAction = "idle" | "cancelling" | "retrying" | "approving" | "rejecting";

export function RunActions({ run }: RunActionsProps) {
	const revalidator = useRevalidator();
	const navigate = useNavigate();
	const [pendingAction, setPendingAction] = useState<PendingAction>("idle");

	const canCancel = run.status === "pending" || run.status === "running" || run.status === "awaiting_approval";
	const canRetry = run.status === "failed" || run.status === "cancelled";
	const canApprove = run.status === "awaiting_approval";
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
			{canApprove && (
				<>
					<Button
						onClick={() =>
							handleAction("approving", () => approveRun(run.id), {
								loading: "Approving run...",
								success: "Run approved",
								error: "Failed to approve run",
							})
						}
						disabled={isLoading}
						variant="default"
						className="bg-emerald-600 hover:bg-emerald-700"
					>
						{pendingAction === "approving" ? "Approving..." : "Approve"}
					</Button>
					<Button
						onClick={() =>
							handleAction("rejecting", () => rejectRun(run.id), {
								loading: "Rejecting run...",
								success: "Run rejected",
								error: "Failed to reject run",
							})
						}
						disabled={isLoading}
						variant="destructive"
					>
						{pendingAction === "rejecting" ? "Rejecting..." : "Reject"}
					</Button>
				</>
			)}
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
