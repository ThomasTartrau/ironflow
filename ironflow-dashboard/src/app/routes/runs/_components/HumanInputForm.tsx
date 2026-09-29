import { useState } from "react";
import { useRevalidator } from "react-router";
import type { JSONSchema7 } from "json-schema";
import type { StepResponse } from "@/app/lib/types";
import { withToast } from "@/app/lib/api-toast";
import {
	validateAgainstSchema,
	extractSchemaProperties,
	buildDefaultValues,
	buildAnswerSkeleton,
	canRenderForm,
} from "@/app/lib/json-schema";
import { formatRemaining, formatEscalationPolicy } from "@/app/lib/format";
import { rejectStepInput, submitStepInput } from "../_actions/actions";
import { MarkdownContent } from "@/app/components/MarkdownContent";
import { HumanInputField } from "./HumanInputField";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Clock } from "lucide-react";

interface HumanInputFormProps {
	step: StepResponse;
}

type PendingAction = "idle" | "submitting" | "rejecting";
type Mode = "form" | "json";

interface StoredInput {
	message: string;
	schema: JSONSchema7 | null;
	escalationPolicy: unknown;
}

/** Read the message, answer schema and escalation policy stored on a human input step. */
function readStoredInput(input: unknown): StoredInput {
	if (typeof input !== "object" || input === null) {
		return { message: "", schema: null, escalationPolicy: undefined };
	}
	const stored = input as Record<string, unknown>;
	const message = typeof stored.message === "string" ? stored.message : "";
	const schema =
		typeof stored.schema === "object" && stored.schema !== null
			? (stored.schema as JSONSchema7)
			: null;
	return { message, schema, escalationPolicy: stored.on_timeout };
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

function DeadlineNotice({
	step,
	escalationPolicy,
}: {
	step: StepResponse;
	escalationPolicy: unknown;
}) {
	if (
		step.approval_deadline_at == null ||
		step.approval_seconds_remaining == null
	) {
		return null;
	}
	const policyLabel = formatEscalationPolicy(escalationPolicy);
	return (
		<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
			<Clock className="w-3 h-3" />
			<span>Expires in {formatRemaining(step.approval_seconds_remaining)}</span>
			{policyLabel && <span>· {policyLabel}</span>}
		</div>
	);
}

/**
 * Answer or reject a human input step waiting on a run.
 *
 * When the schema's top-level properties can each be rendered as a widget,
 * shows a generated form with a toggle to the raw JSON editor. Otherwise
 * falls back to the JSON editor alone. Either way the answer is checked
 * against the schema before it can be submitted; the server validates it
 * again and stays authoritative.
 */
export function HumanInputForm({ step }: HumanInputFormProps) {
	const revalidator = useRevalidator();
	const { message, schema, escalationPolicy } = readStoredInput(step.input);
	const { properties, requiredFields } = extractSchemaProperties(schema);
	const fieldNames = Object.keys(properties);
	const formCapable =
		schema !== null && fieldNames.length > 0 && canRenderForm(properties);

	const [mode, setMode] = useState<Mode>(formCapable ? "form" : "json");
	const [formValues, setFormValues] = useState<Record<string, unknown>>(() =>
		formCapable ? buildDefaultValues(properties) : {},
	);
	const [raw, setRaw] = useState<string>(() =>
		JSON.stringify(
			formCapable
				? buildDefaultValues(properties)
				: buildAnswerSkeleton(schema),
			null,
			2,
		),
	);
	const [touched, setTouched] = useState(false);
	const [reason, setReason] = useState("");
	const [pendingAction, setPendingAction] = useState<PendingAction>("idle");
	const isLoading = pendingAction !== "idle";

	const jsonParsed = parseAnswer(raw, schema);
	const currentValue = mode === "form" ? formValues : jsonParsed.value;
	const currentErrors =
		mode === "form"
			? schema
				? validateAgainstSchema(formValues, schema)
				: []
			: jsonParsed.errors;
	const isValid = currentErrors.length === 0;
	const showErrors = touched && !isValid;

	const switchToJson = () => {
		setRaw(JSON.stringify(formValues, null, 2));
		setMode("json");
	};
	const switchToForm = () => {
		try {
			const parsedValue = JSON.parse(raw);
			if (
				typeof parsedValue === "object" &&
				parsedValue !== null &&
				!Array.isArray(parsedValue)
			) {
				setFormValues(parsedValue as Record<string, unknown>);
			}
		} catch {
			// keep the current form values
		}
		setMode("form");
	};

	const updateField = (key: string, value: unknown) => {
		setTouched(true);
		setFormValues((prev) => ({ ...prev, [key]: value }));
	};

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
			submitStepInput(step.run_id, step.id, currentValue),
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
			{message && <MarkdownContent content={message} />}
			<DeadlineNotice step={step} escalationPolicy={escalationPolicy} />
			{formCapable && (
				<div className="inline-flex rounded-[var(--radius-sm)] border border-border overflow-hidden text-xs w-fit">
					<button
						type="button"
						onClick={switchToForm}
						aria-pressed={mode === "form"}
						className={`px-2 py-1 ${mode === "form" ? "bg-primary text-primary-foreground" : "bg-transparent text-muted-foreground hover:bg-muted"}`}
					>
						Form
					</button>
					<button
						type="button"
						onClick={switchToJson}
						aria-pressed={mode === "json"}
						className={`px-2 py-1 ${mode === "json" ? "bg-primary text-primary-foreground" : "bg-transparent text-muted-foreground hover:bg-muted"}`}
					>
						JSON
					</button>
				</div>
			)}
			<fieldset disabled={isLoading} className="space-y-3">
				{mode === "form" && formCapable ? (
					<div className="space-y-3">
						{fieldNames.map((key) => (
							<HumanInputField
								key={key}
								name={key}
								schema={properties[key]}
								value={formValues[key]}
								onChange={(v) => updateField(key, v)}
								required={requiredFields.has(key)}
							/>
						))}
					</div>
				) : (
					<Textarea
						aria-label="Answer (JSON)"
						className="font-mono text-xs"
						value={raw}
						onChange={(e) => {
							setTouched(true);
							setRaw(e.target.value);
						}}
						aria-invalid={!isValid}
						rows={6}
					/>
				)}
			</fieldset>
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
			{showErrors && (
				<ul className="list-disc pl-5 text-xs text-destructive">
					{currentErrors.map((error) => (
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
