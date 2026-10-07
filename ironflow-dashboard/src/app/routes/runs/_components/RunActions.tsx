import { useState } from "react";
import { useNavigate, useRevalidator } from "react-router";
import type { RunResponse } from "@/app/lib/types";
import { withToast, type ToastMessages } from "@/app/lib/api-toast";
import {
	approveRun,
	cancelRun,
	pauseRun,
	rejectRun,
	replayRun,
	resumeRun,
	retryRun,
} from "../_actions/actions";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { useAppSelector } from "@/app/store";

interface RunActionsProps {
	run: RunResponse;
	/** A human input step is open: it is answered there, not approved. */
	awaitingInput: boolean;
	/** Sub-workflow runs below this run that a cancellation also stops. */
	activeDescendantCount: number;
}

/** What a cancellation reaches below the run, for the confirmation. */
function descendantsNotice(count: number): string {
	if (count === 0) return "No sub-run is active.";
	const noun = count === 1 ? "sub-run" : "sub-runs";
	return `${count} active ${noun} will be cancelled with it.`;
}

type PendingAction =
	| "idle"
	| "cancelling"
	| "pausing"
	| "resuming"
	| "retrying"
	| "replaying"
	| "approving"
	| "rejecting";

export function RunActions({
	run,
	awaitingInput,
	activeDescendantCount,
}: RunActionsProps) {
	const revalidator = useRevalidator();
	const navigate = useNavigate();
	const [pendingAction, setPendingAction] = useState<PendingAction>("idle");
	const [confirmingCancel, setConfirmingCancel] = useState(false);
	const auth = useAppSelector((state) => state.auth);
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;

	if (!isAdmin) return null;

	const canCancel =
		run.status === "pending" ||
		run.status === "running" ||
		run.status === "awaiting_approval" ||
		run.status === "sleeping" ||
		run.status === "paused";
	// A sub-workflow run is paused and resumed through its root run.
	const isSubRun = run.trigger.kind === "workflow";
	const canPause =
		!isSubRun &&
		(run.status === "pending" ||
			run.status === "running" ||
			run.status === "retrying" ||
			run.status === "awaiting_approval" ||
			run.status === "sleeping");
	const canResume = !isSubRun && run.status === "paused";
	const canRetry = run.status === "failed" || run.status === "cancelled";
	const canReplay = run.status === "completed" || run.status === "failed";
	const canApprove = run.status === "awaiting_approval" && !awaitingInput;
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
						className="bg-success text-success-foreground hover:bg-success/90"
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
			{canPause && (
				<Button
					onClick={() =>
						handleAction("pausing", () => pauseRun(run.id), {
							loading: "Pausing run...",
							success: "Run paused",
							error: "Failed to pause run",
						})
					}
					disabled={isLoading}
					variant="outline"
				>
					{pendingAction === "pausing" ? "Pausing..." : "Pause"}
				</Button>
			)}
			{canResume && (
				<Button
					onClick={() =>
						handleAction("resuming", () => resumeRun(run.id), {
							loading: "Resuming run...",
							success: "Run resumed",
							error: "Failed to resume run",
						})
					}
					disabled={isLoading}
					variant="default"
				>
					{pendingAction === "resuming" ? "Resuming..." : "Resume"}
				</Button>
			)}
			{canCancel && (
				<Button
					onClick={() => setConfirmingCancel(true)}
					disabled={isLoading}
					variant="outline"
					className="border-destructive text-destructive hover:bg-destructive/10"
				>
					{pendingAction === "cancelling" ? "Cancelling..." : "Cancel"}
				</Button>
			)}
			<Dialog open={confirmingCancel} onOpenChange={setConfirmingCancel}>
				<DialogContent showCloseButton={false}>
					<DialogHeader>
						<DialogTitle>Cancel this run?</DialogTitle>
						<DialogDescription>
							{descendantsNotice(activeDescendantCount)}
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setConfirmingCancel(false)}
							type="button"
						>
							Keep running
						</Button>
						<Button
							variant="destructive"
							type="button"
							onClick={() => {
								setConfirmingCancel(false);
								handleAction("cancelling", () => cancelRun(run.id), {
									loading: "Cancelling run...",
									success: "Run cancelled",
									error: "Failed to cancel run",
								});
							}}
						>
							Cancel run
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
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
			{canReplay && (
				<Button
					onClick={() =>
						handleAction(
							"replaying",
							() => replayRun(run.id),
							{
								loading: "Replaying run...",
								success: "Run replayed",
								error: "Failed to replay run",
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
					{pendingAction === "replaying" ? "Replaying..." : "Replay"}
				</Button>
			)}
		</div>
	);
}
