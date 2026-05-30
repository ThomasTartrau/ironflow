import { useState } from "react";
import type {
	WorkflowDetailResponse,
	RunResponse,
	CreateRunRequest,
} from "@/app/lib/types";
import { api } from "@/app/lib/api";
import { withToast } from "@/app/lib/api-toast";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Play, Plus, X, Clock } from "lucide-react";
import type { JSONSchema7, JSONSchema7Definition } from "json-schema";
import { SchemaField } from "./SchemaField";

function isJsonSchema(value: unknown): value is JSONSchema7 {
	return (
		typeof value === "object" &&
		value !== null &&
		"type" in value &&
		(value as JSONSchema7).type === "object"
	);
}

function resolveProperty(def: JSONSchema7Definition): JSONSchema7 | null {
	if (typeof def === "boolean") return null;
	return def;
}

interface LabelEntry {
	id: string;
	key: string;
	value: string;
}

interface RunDialogProps {
	workflow: WorkflowDetailResponse;
	onCreated: (runId: string) => void;
}

export function RunDialog({ workflow, onCreated }: RunDialogProps) {
	const [open, setOpen] = useState(false);
	const [loading, setLoading] = useState(false);

	const schema = isJsonSchema(workflow.input_schema)
		? workflow.input_schema
		: null;
	const rawProperties = schema?.properties ?? {};
	const requiredFields = new Set(schema?.required ?? []);

	const properties: Record<string, JSONSchema7> = {};
	for (const [key, def] of Object.entries(rawProperties)) {
		const resolved = resolveProperty(def);
		if (resolved) properties[key] = resolved;
	}

	const [formValues, setFormValues] = useState<Record<string, unknown>>(() => {
		const initial: Record<string, unknown> = {};
		for (const [key, prop] of Object.entries(properties)) {
			if (prop.default != null) {
				initial[key] = prop.default;
			}
		}
		return initial;
	});

	const [labels, setLabels] = useState<LabelEntry[]>(() => {
		const defaults = workflow.default_labels;
		if (!defaults) return [];
		return Object.entries(defaults).map(([key, value]) => ({
			id: crypto.randomUUID(),
			key,
			value,
		}));
	});
	const [scheduleEnabled, setScheduleEnabled] = useState(false);
	const [scheduledAt, setScheduledAt] = useState("");

	const updateField = (key: string, value: unknown) => {
		setFormValues((prev) => ({ ...prev, [key]: value }));
	};

	const addLabel = () => {
		setLabels((prev) => [
			...prev,
			{ id: crypto.randomUUID(), key: "", value: "" },
		]);
	};

	const removeLabel = (id: string) => {
		setLabels((prev) => prev.filter((entry) => entry.id !== id));
	};

	const updateLabel = (id: string, field: "key" | "value", value: string) => {
		setLabels((prev) =>
			prev.map((entry) =>
				entry.id === id ? { ...entry, [field]: value } : entry,
			),
		);
	};

	const missingRequired = schema
		? [...requiredFields].filter((key) => {
				const val = formValues[key];
				return val === undefined || val === null || val === "";
			})
		: [];

	const handleSubmit = () => {
		if (missingRequired.length > 0) return;

		const payload: Record<string, unknown> = schema ? { ...formValues } : {};

		const labelsMap: Record<string, string> = {};
		for (const entry of labels) {
			if (entry.key.trim()) {
				labelsMap[entry.key.trim()] = entry.value;
			}
		}

		const request: CreateRunRequest = {
			workflow: workflow.name,
			payload,
			labels: Object.keys(labelsMap).length > 0 ? labelsMap : undefined,
			scheduled_at:
				scheduleEnabled && scheduledAt
					? new Date(scheduledAt).toISOString()
					: undefined,
		};

		setLoading(true);
		withToast(api.post<RunResponse>("/runs", request), {
			loading: scheduleEnabled ? "Scheduling run..." : "Starting run...",
			success: scheduleEnabled ? "Run scheduled!" : "Run started!",
			error: "Failed to start run",
		})
			.then((res) => {
				setOpen(false);
				onCreated(res.data.id);
			})
			.catch(() => {})
			.finally(() => setLoading(false));
	};

	const fieldNames = Object.keys(properties);

	return (
		<Dialog open={open} onOpenChange={loading ? undefined : setOpen}>
			<DialogTrigger
				render={
					<Button className="w-full sm:w-fit gap-1.5">
						<Play className="size-4" aria-hidden="true" />
						Run
					</Button>
				}
			/>
			<DialogContent className="max-w-lg max-h-[85vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle>
						Run <span className="font-mono">{workflow.name}</span>
					</DialogTitle>
					<DialogDescription>
						{schema
							? "Fill in the parameters below."
							: "This workflow takes no input parameters. Click Run now to start it."}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-5 py-2">
					{schema && fieldNames.length > 0 && (
						<div className="space-y-4">
							{fieldNames.map((key) => (
								<SchemaField
									key={key}
									name={key}
									schema={properties[key]}
									value={formValues[key]}
									onChange={(v) => updateField(key, v)}
									required={requiredFields.has(key)}
								/>
							))}
						</div>
					)}

					<div className="space-y-3">
						<div className="flex items-center justify-between">
							<span className="text-sm font-medium">Labels</span>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="gap-1 h-7 text-xs"
								onClick={addLabel}
							>
								<Plus className="size-3" aria-hidden="true" />
								Add
							</Button>
						</div>
						{labels.map((entry, index) => (
							<div key={entry.id} className="flex gap-2 items-center">
								<Input
									placeholder="key"
									className="text-sm font-mono"
									value={entry.key}
									onChange={(e) => updateLabel(entry.id, "key", e.target.value)}
								/>
								<Input
									placeholder="value"
									className="text-sm"
									value={entry.value}
									onChange={(e) =>
										updateLabel(entry.id, "value", e.target.value)
									}
								/>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									className="size-8 shrink-0"
									onClick={() => removeLabel(entry.id)}
									aria-label={`Remove label ${entry.key || String(index + 1)}`}
								>
									<X className="size-3.5" aria-hidden="true" />
								</Button>
							</div>
						))}
					</div>

					<div className="space-y-3">
						<div className="flex items-center justify-between">
							<div className="flex items-center gap-2">
								<Clock
									className="size-4 text-muted-foreground"
									aria-hidden="true"
								/>
								<span className="text-sm font-medium">Schedule for later</span>
							</div>
							<Switch
								checked={scheduleEnabled}
								onCheckedChange={setScheduleEnabled}
							/>
						</div>
						{scheduleEnabled && (
							<Input
								type="datetime-local"
								value={scheduledAt}
								onChange={(e) => setScheduledAt(e.target.value)}
								min={new Date().toISOString().slice(0, 16)}
							/>
						)}
					</div>
				</div>

				{missingRequired.length > 0 && (
					<p className="text-xs text-destructive mt-1">
						Required fields: {missingRequired.join(", ")}
					</p>
				)}

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => setOpen(false)}
						disabled={loading}
					>
						Cancel
					</Button>
					<Button
						onClick={handleSubmit}
						disabled={loading || missingRequired.length > 0}
						className="gap-1.5"
					>
						<Play className="size-4" aria-hidden="true" />
						{loading ? "Starting..." : scheduleEnabled ? "Schedule" : "Run now"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
