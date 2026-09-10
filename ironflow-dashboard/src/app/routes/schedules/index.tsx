import { useCallback, useState, type FormEvent } from "react";
import { useLoaderData, useRevalidator } from "react-router";
import {
	CalendarClock,
	Loader2,
	Pause,
	Play,
	Plus,
	Trash2,
	Zap,
} from "lucide-react";
import type { JSONSchema7 } from "json-schema";
import { api } from "@/app/lib/api";
import {
	isJsonSchema,
	extractSchemaProperties,
	buildDefaultValues,
} from "@/app/lib/json-schema";
import { HeaderApp } from "@/app/components/HeaderApp";
import { SchemaField } from "@/app/components/SchemaField";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { TimeAgo } from "@/app/components/TimeAgo";
import { withToast } from "@/app/lib/api-toast";

interface ScheduleResponse {
	id: string;
	workflow_name: string;
	cron_expression: string;
	inputs: Record<string, unknown>;
	source: "handler" | "api";
	disabled_at: string | null;
	last_triggered_at: string | null;
	next_trigger_at: string | null;
	created_by_user_id: string;
	created_at: string;
	updated_at: string;
}

interface WorkflowSummary {
	name: string;
}

export async function loader() {
	const [schedulesRes, workflowsRes] = await Promise.all([
		api.get<ScheduleResponse[]>("/schedules"),
		api.get<WorkflowSummary[]>("/workflows"),
	]);
	return {
		schedules: schedulesRes.data,
		workflows: workflowsRes.data,
	};
}

export function Component() {
	const { schedules, workflows } = useLoaderData() as {
		schedules: ScheduleResponse[];
		workflows: WorkflowSummary[];
	};
	const revalidator = useRevalidator();
	const [createOpen, setCreateOpen] = useState(false);
	const [deleting, setDeleting] = useState<string | null>(null);
	const [pendingDelete, setPendingDelete] = useState<string | null>(null);

	const [newWorkflow, setNewWorkflow] = useState("");
	const [newCron, setNewCron] = useState("");
	const [creating, setCreating] = useState(false);

	const [schema, setSchema] = useState<JSONSchema7 | null>(null);
	const [schemaLoading, setSchemaLoading] = useState(false);
	const [formValues, setFormValues] = useState<Record<string, unknown>>({});

	const { properties, requiredFields } = extractSchemaProperties(schema);
	const fieldNames = Object.keys(properties);

	const fetchSchema = useCallback(async (workflowName: string) => {
		setSchemaLoading(true);
		try {
			const res = await api.get<{ input_schema?: unknown }>(
				`/workflows/${encodeURIComponent(workflowName)}`,
			);
			const raw = res.data.input_schema;
			const parsed = isJsonSchema(raw) ? (raw as JSONSchema7) : null;
			setSchema(parsed);
			setFormValues(
				parsed
					? buildDefaultValues(extractSchemaProperties(parsed).properties)
					: {},
			);
		} catch {
			setSchema(null);
			setFormValues({});
		} finally {
			setSchemaLoading(false);
		}
	}, []);

	function handleWorkflowChange(name: string) {
		setNewWorkflow(name);
		if (name) {
			fetchSchema(name);
		} else {
			setSchema(null);
			setFormValues({});
		}
	}

	const updateField = (key: string, value: unknown) => {
		setFormValues((prev) => ({ ...prev, [key]: value }));
	};

	const missingRequired = schema
		? [...requiredFields].filter((key) => {
				const val = formValues[key];
				return val === undefined || val === null || val === "";
			})
		: [];

	useDocumentMeta({
		title: "Schedules",
		description: "Manage periodic workflow schedules.",
	});

	async function handleCreate(e: FormEvent) {
		e.preventDefault();
		if (missingRequired.length > 0) return;
		setCreating(true);
		const inputs: Record<string, unknown> = schema ? { ...formValues } : {};
		await withToast(
			api.post("/schedules", {
				workflow_name: newWorkflow,
				cron_expression: newCron,
				inputs,
			}),
			{
				loading: "Creating schedule...",
				success: "Schedule created",
				error: "Failed to create schedule",
			},
		);
		setCreating(false);
		setCreateOpen(false);
		setNewWorkflow("");
		setNewCron("");
		setSchema(null);
		setFormValues({});
		revalidator.revalidate();
	}

	async function toggleEnabled(schedule: ScheduleResponse) {
		const isActive = schedule.disabled_at === null;
		const action = isActive ? "pause" : "resume";
		await withToast(api.post(`/schedules/${schedule.id}/${action}`), {
			loading: isActive ? "Pausing..." : "Resuming...",
			success: isActive ? "Schedule paused" : "Schedule resumed",
			error: `Failed to ${action} schedule`,
		});
		revalidator.revalidate();
	}

	async function handleTrigger(id: string) {
		await withToast(api.post(`/schedules/${id}/trigger`), {
			loading: "Triggering...",
			success: "Run created",
			error: "Failed to trigger schedule",
		});
		revalidator.revalidate();
	}

	async function handleDelete(id: string) {
		setDeleting(id);
		await withToast(api.del(`/schedules/${id}`), {
			loading: "Deleting...",
			success: "Schedule deleted",
			error: "Failed to delete schedule",
		});
		setDeleting(null);
		setPendingDelete(null);
		revalidator.revalidate();
	}

	return (
		<>
			<HeaderApp
				title="Schedules"
				description="Manage periodic workflow schedules."
			>
				<Button size="sm" onClick={() => setCreateOpen(true)}>
					<Plus className="size-4 mr-1" />
					New Schedule
				</Button>
			</HeaderApp>

			<div className="p-6">
				{schedules.length === 0 ? (
					<div className="text-center py-12 text-muted-foreground">
						<CalendarClock className="size-12 mx-auto mb-4 opacity-30" />
						<p>No schedules yet.</p>
						<p className="text-sm mt-1">
							Create one to trigger workflows on a cron expression.
						</p>
					</div>
				) : (
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Workflow</TableHead>
								<TableHead>Cron</TableHead>
								<TableHead>Source</TableHead>
								<TableHead>Next trigger</TableHead>
								<TableHead>Last trigger</TableHead>
								<TableHead>Status</TableHead>
								<TableHead className="text-right">Actions</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{schedules.map((s) => (
								<TableRow key={s.id}>
									<TableCell className="font-medium">
										{s.workflow_name}
									</TableCell>
									<TableCell>
										<code className="text-xs bg-muted px-1.5 py-0.5 rounded">
											{s.cron_expression}
										</code>
									</TableCell>
									<TableCell>
										<Badge variant={s.source === "handler" ? "outline" : "secondary"}>
											{s.source === "handler" ? "Code" : "API"}
										</Badge>
									</TableCell>
									<TableCell>
										{s.next_trigger_at ? (
											<TimeAgo date={s.next_trigger_at} />
										) : (
											<span className="text-muted-foreground">-</span>
										)}
									</TableCell>
									<TableCell>
										{s.last_triggered_at ? (
											<TimeAgo date={s.last_triggered_at} />
										) : (
											<span className="text-muted-foreground">Never</span>
										)}
									</TableCell>
									<TableCell>
										<Badge
											variant={s.disabled_at === null ? "default" : "secondary"}
											className="cursor-pointer"
											onClick={() => toggleEnabled(s)}
										>
											{s.disabled_at === null ? "Active" : "Paused"}
										</Badge>
									</TableCell>
									<TableCell className="text-right space-x-1">
										<TooltipProvider>
											<Tooltip>
												<TooltipTrigger
													render={
														<Button
															variant="ghost"
															size="icon"
															onClick={() => toggleEnabled(s)}
															aria-label={
																s.disabled_at === null ? "Pause" : "Resume"
															}
														>
															{s.disabled_at === null ? (
																<Pause className="size-4" />
															) : (
																<Play className="size-4" />
															)}
														</Button>
													}
												/>
												<TooltipContent>
													{s.disabled_at === null ? "Pause" : "Resume"}
												</TooltipContent>
											</Tooltip>
										</TooltipProvider>
										<TooltipProvider>
											<Tooltip>
												<TooltipTrigger
													render={
														<Button
															variant="ghost"
															size="icon"
															onClick={() => handleTrigger(s.id)}
															aria-label="Trigger now"
														>
															<Zap className="size-4" />
														</Button>
													}
												/>
												<TooltipContent>Trigger now</TooltipContent>
											</Tooltip>
										</TooltipProvider>
										{s.source !== "handler" && (
											<TooltipProvider>
												<Tooltip>
													<TooltipTrigger
														render={
															<Button
																variant="ghost"
																size="icon"
																onClick={() => setPendingDelete(s.id)}
																aria-label="Delete"
															>
																<Trash2 className="size-4" />
															</Button>
														}
													/>
													<TooltipContent>Delete</TooltipContent>
												</Tooltip>
											</TooltipProvider>
										)}
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				)}
			</div>

			<Dialog open={createOpen} onOpenChange={setCreateOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>New Schedule</DialogTitle>
						<DialogDescription>
							Create a recurring schedule for a workflow.
						</DialogDescription>
					</DialogHeader>
					<form onSubmit={handleCreate} className="space-y-4">
						<div className="space-y-2">
							<label htmlFor="workflow" className="text-sm font-medium">
								Workflow
							</label>
							<Select
								value={newWorkflow}
								onValueChange={(v) => v && handleWorkflowChange(v)}
							>
								<SelectTrigger id="workflow">
									<SelectValue placeholder="Select a workflow" />
								</SelectTrigger>
								<SelectContent>
									{workflows.map((w) => (
										<SelectItem key={w.name} value={w.name}>
											{w.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-2">
							<label htmlFor="cron" className="text-sm font-medium">
								Cron expression
							</label>
							<Input
								id="cron"
								placeholder="*/5 * * * *"
								value={newCron}
								onChange={(e) => setNewCron(e.target.value)}
							/>
							<p className="text-xs text-muted-foreground">
								Standard 5-field format (min hour dom month dow). 6-field with
								seconds also accepted.
							</p>
						</div>
						{schemaLoading && (
							<div className="flex items-center gap-2 text-sm text-muted-foreground">
								<Loader2 className="size-4 animate-spin" />
								Loading parameters...
							</div>
						)}
						{!schemaLoading && schema && fieldNames.length > 0 && (
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
						{!schemaLoading && newWorkflow && !schema && (
							<p className="text-sm text-muted-foreground">
								This workflow takes no input parameters.
							</p>
						)}
						{missingRequired.length > 0 && (
							<p className="text-xs text-destructive">
								Required fields: {missingRequired.join(", ")}
							</p>
						)}
						<DialogFooter>
							<Button
								type="submit"
								disabled={
									!newWorkflow ||
									!newCron ||
									creating ||
									schemaLoading ||
									missingRequired.length > 0
								}
							>
								{creating ? "Creating..." : "Create"}
							</Button>
						</DialogFooter>
					</form>
				</DialogContent>
			</Dialog>

			<Dialog
				open={pendingDelete !== null}
				onOpenChange={() => setPendingDelete(null)}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete schedule?</DialogTitle>
						<DialogDescription>
							This action cannot be undone. Existing runs will not be affected.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setPendingDelete(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							disabled={deleting !== null}
							onClick={() => pendingDelete && handleDelete(pendingDelete)}
						>
							{deleting ? "Deleting..." : "Delete"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
}
