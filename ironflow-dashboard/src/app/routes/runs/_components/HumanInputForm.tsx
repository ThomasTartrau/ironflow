import { useState } from "react";
import { useRevalidator } from "react-router";
import type { JSONSchema7 } from "json-schema";
import type { StepResponse } from "@/app/lib/types";
import { withToast } from "@/app/lib/api-toast";
import { validateAgainstSchema } from "@/app/lib/json-schema";
import { rejectStepInput, submitStepInput } from "../_actions/actions";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";

interface HumanInputFormProps {
	step: StepResponse;
}

type PendingAction = "idle" | "submitting" | "rejecting";

interface StoredInput {
	message: string;
	schema: JSONSchema7 | null;
}

/** Read the message and the answer schema stored on a human input step. */
function readStoredInput(input: unknown): StoredInput {
	if (typeof input !== "object" || input === null) {
		return { message: "", schema: null };
	}
	const stored = input as Record<string, unknown>;
	const message = typeof stored.message === "string" ? stored.message : "";
	const schema =
		typeof stored.schema === "object" && stored.schema !== null
			? (stored.schema as JSONSchema7)
			: null;
	return { message, schema };
}

interface ParsedAnswer {
	value: unknown;
	errors: string[];
}

/** Parse the typed JSON and check it against the schema. */
function parseAnswer(raw: string, schema: JSONSchema7 | null): ParsedAnswer {
	let value: unknown;
	try {
		value = JSON.parse(raw);
	} catch (err) {
		const detail = err instanceof Error ? err.message : String(err);
		return { value: undefined, errors: [`Invalid JSON: ${detail}`] };
	}
	const errors = schema ? validateAgainstSchema(value, schema) : [];
	return { value, errors };
}

/**
 * Answer or reject a human input step waiting on a run.
 *
 * The answer is typed as JSON and checked against the schema stored on the
 * step before it can be submitted. The server validates it again and stays
 * authoritative.
 */
export function HumanInputForm({ step }: HumanInputFormProps) {
	const revalidator = useRevalidator();
	const [raw, setRaw] = useState("{}");
	const [reason, setReason] = useState("");
	const [pendingAction, setPendingAction] = useState<PendingAction>("idle");

	const { message, schema } = readStoredInput(step.input);
	const parsed = parseAnswer(raw, schema);
	const isValid = parsed.errors.length === 0;
	const isLoading = pendingAction !== "idle";

	const run = (action: PendingAction, promise: () => Promise<unknown>) => {
		setPendingAction(action);
		const messages =
			action === "submitting"
				? {
						loading: "Submitting answer...",
						success: "Answer submitted",
						error: "Failed to submit answer",
					}
				: {
						loading: "Rejecting input...",
						success: "Input rejected",
						error: "Failed to reject input",
					};
		withToast(promise(), messages)
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setPendingAction("idle"));
	};

	const handleSubmit = () =>
		run("submitting", () =>
			submitStepInput(step.run_id, step.id, parsed.value),
		);
	const handleReject = () =>
		run("rejecting", () =>
			rejectStepInput(step.run_id, step.id, reason.trim() || undefined),
		);

	return (
		<div className="space-y-3 rounded-[var(--radius-sm)] border border-cyan-400/40 bg-cyan-400/5 p-3">
			<div className="text-xs font-semibold text-cyan-700 dark:text-cyan-300">
				Waiting for input: {step.name}
			</div>
			{message && (
				<div className="text-sm whitespace-pre-wrap break-words">{message}</div>
			)}
			{schema && (
				<details className="text-xs">
					<summary className="cursor-pointer text-muted-foreground">
						Expected schema
					</summary>
					<pre className="mt-2 max-h-64 overflow-auto rounded-[var(--radius-sm)] bg-muted/50 p-2 font-mono">
						{JSON.stringify(schema, null, 2)}
					</pre>
				</details>
			)}
			<Textarea
				aria-label="Answer (JSON)"
				className="font-mono text-xs"
				value={raw}
				onChange={(e) => setRaw(e.target.value)}
				aria-invalid={!isValid}
				disabled={isLoading}
				rows={6}
			/>
			{!isValid && (
				<ul className="list-disc pl-5 text-xs text-destructive">
					{parsed.errors.map((error) => (
						<li key={error}>{error}</li>
					))}
				</ul>
			)}
			<div className="flex flex-wrap items-center gap-2">
				<Button onClick={handleSubmit} disabled={!isValid || isLoading}>
					{pendingAction === "submitting" ? "Submitting..." : "Submit"}
				</Button>
				<Input
					aria-label="Rejection reason"
					placeholder="Reason (optional)"
					className="max-w-xs"
					value={reason}
					onChange={(e) => setReason(e.target.value)}
					disabled={isLoading}
				/>
				<Button
					variant="destructive"
					onClick={handleReject}
					disabled={isLoading}
				>
					{pendingAction === "rejecting" ? "Rejecting..." : "Reject"}
				</Button>
			</div>
		</div>
	);
}
