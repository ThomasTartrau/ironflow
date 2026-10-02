import { useState } from "react";
import { useRevalidator } from "react-router";
import type { JSONSchema7 } from "json-schema";
import type { StepResponse } from "@/app/lib/types";
import { withToast } from "@/app/lib/api-toast";
import { validateAgainstSchema } from "@/app/lib/json-schema";
import { useAppSelector } from "@/app/store";
import { sendSignal } from "../_actions/actions";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Radio } from "lucide-react";

interface SignalStepPanelProps {
	step: StepResponse;
}

interface StoredSignalInput {
	name: string;
	key: string;
	schema: JSONSchema7 | null;
	waitingSince: string | null;
	deadlineAt: string | null;
}

/** Read the signal name, key, payload schema and timing stored on a signal step. */
function readStoredInput(input: unknown): StoredSignalInput {
	const stored =
		typeof input === "object" && input !== null
			? (input as Record<string, unknown>)
			: {};
	const text = (value: unknown) => (typeof value === "string" ? value : null);
	return {
		name: text(stored.name) ?? "",
		key: text(stored.key) ?? "",
		schema:
			typeof stored.schema === "object" && stored.schema !== null
				? (stored.schema as JSONSchema7)
				: null,
		waitingSince: text(stored.waiting_since),
		deadlineAt: text(stored.deadline_at),
	};
}

/** Parse the typed payload and check it against the schema. */
function parsePayload(raw: string, schema: JSONSchema7 | null): string[] {
	let value: unknown;
	try {
		value = JSON.parse(raw);
	} catch (err) {
		const detail = err instanceof Error ? err.message : String(err);
		return [`Invalid JSON: ${detail}`];
	}
	return schema ? validateAgainstSchema(value, schema) : [];
}

/**
 * Show what a signal step waits for and, for an admin, deliver the signal by
 * hand. The payload is checked against the stored schema before it can be
 * sent; the server validates it again and stays authoritative.
 */
export function SignalStepPanel({ step }: SignalStepPanelProps) {
	const revalidator = useRevalidator();
	const auth = useAppSelector((state) => state.auth);
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;
	const { name, key, schema, waitingSince, deadlineAt } = readStoredInput(
		step.input,
	);
	const [raw, setRaw] = useState("{}");
	const [sending, setSending] = useState(false);

	const errors = parsePayload(raw, schema);
	const waiting = step.status === "running";

	async function handleSend() {
		setSending(true);
		try {
			await withToast(sendSignal(name, key, JSON.parse(raw)), {
				loading: "Delivering signal...",
				success: "Signal delivered",
				error: "Failed to deliver the signal",
			});
			revalidator.revalidate();
		} finally {
			setSending(false);
		}
	}

	return (
		<div className="p-3 rounded-[var(--radius-sm)] bg-muted/50 border border-border space-y-2">
			<div className="flex items-center gap-1.5 text-xs font-semibold text-muted-foreground">
				<Radio className="w-3 h-3" />
				<span>Signal</span>
			</div>
			<dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
				<dt className="text-muted-foreground">Name</dt>
				<dd className="font-mono">{name}</dd>
				<dt className="text-muted-foreground">Key</dt>
				<dd className="font-mono">{key}</dd>
				{waitingSince && (
					<>
						<dt className="text-muted-foreground">Waiting since</dt>
						<dd>{waitingSince}</dd>
					</>
				)}
				{deadlineAt && (
					<>
						<dt className="text-muted-foreground">Deadline</dt>
						<dd>{deadlineAt}</dd>
					</>
				)}
			</dl>
			{waiting && isAdmin && (
				<div className="space-y-2">
					<Textarea
						aria-label="Signal payload"
						className="font-mono text-xs"
						value={raw}
						onChange={(event) => setRaw(event.target.value)}
					/>
					{errors.length > 0 && (
						<ul className="text-xs text-destructive">
							{errors.map((error) => (
								<li key={error}>{error}</li>
							))}
						</ul>
					)}
					<Button
						size="sm"
						disabled={sending || errors.length > 0}
						onClick={handleSend}
					>
						Deliver signal
					</Button>
				</div>
			)}
		</div>
	);
}
