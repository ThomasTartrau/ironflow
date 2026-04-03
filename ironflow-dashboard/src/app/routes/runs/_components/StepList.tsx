import { useState, useEffect, useCallback } from "react";
import { Link } from "react-router";
import type {
	StepResponse,
	RunDetailResponse,
	RunStatus,
} from "@/app/lib/types";

declare global {
	interface Window {
		__focusStepTarget: string | null;
	}
}
import { api } from "@/app/lib/api";
import { StatusBadge } from "@/app/components/StatusBadge";
import { MarkdownContent } from "@/app/components/MarkdownContent";
import { Badge } from "@/components/ui/badge";
import {
	Collapsible,
	CollapsibleTrigger,
	CollapsibleContent,
} from "@/components/ui/collapsible";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { formatDuration, formatCost } from "@/app/lib/format";
import {
	ChevronDown,
	ChevronUp,
	Clock,
	DollarSign,
	Cpu,
	Hash,
} from "lucide-react";

interface StepListProps {
	steps: StepResponse[];
}

function getKindColor(kind: string): string {
	switch (kind) {
		case "shell":
			return "bg-amber-100 text-amber-700 border-amber-200";
		case "http":
			return "bg-sky-100 text-sky-700 border-sky-200";
		case "agent":
			return "bg-violet-100 text-violet-700 border-violet-200";
		case "workflow":
			return "bg-indigo-100 text-indigo-700 border-indigo-200";
		case "approval":
			return "bg-rose-100 text-rose-700 border-rose-200";
		default:
			return "bg-gray-100 text-gray-700 border-gray-200";
	}
}

function ChildRunSteps({ runId }: { runId: string }) {
	const [steps, setSteps] = useState<StepResponse[] | null>(null);
	const [error, setError] = useState(false);

	useEffect(() => {
		api
			.get<RunDetailResponse>(`/runs/${runId}`)
			.then((res) => setSteps(res.data.steps))
			.catch(() => setError(true));
	}, [runId]);

	if (error) {
		return (
			<div className="text-xs text-muted-foreground italic">
				Failed to load child run steps
			</div>
		);
	}

	if (!steps) {
		return (
			<div className="text-xs text-muted-foreground italic">
				Loading steps...
			</div>
		);
	}

	if (steps.length === 0) return null;

	return (
		<div className="border-l-2 border-indigo-200 pl-4 space-y-2">
			{steps.map((childStep) => (
				<NestedStep key={childStep.id} step={childStep} />
			))}
		</div>
	);
}

function NestedStep({ step }: { step: StepResponse }) {
	const [isExpanded, setIsExpanded] = useState(false);
	const [highlight, setHighlight] = useState(false);

	const focusThis = useCallback(() => {
		setIsExpanded(true);
		setHighlight(true);
		setTimeout(() => setHighlight(false), 1500);
	}, []);

	useEffect(() => {
		const target = window.__focusStepTarget;
		if (target === step.id) {
			focusThis();
		} else if (target && step.kind === "workflow") {
			setIsExpanded(true);
		}

		const handler = (e: Event) => {
			const detail = (e as CustomEvent).detail;
			const targetId = typeof detail === "string" ? detail : detail?.stepId;
			if (targetId === step.id) {
				focusThis();
			} else if (targetId && step.kind === "workflow") {
				setIsExpanded(true);
			}
		};
		window.addEventListener("focus-step", handler);
		window.addEventListener("focus-step-nested", handler);
		return () => {
			window.removeEventListener("focus-step", handler);
			window.removeEventListener("focus-step-nested", handler);
		};
	}, [step.id, step.kind, focusThis]);

	return (
		<div id={`step-${step.id}`} className="space-y-2">
			<button
				type="button"
				onClick={() => setIsExpanded(!isExpanded)}
				className={`flex items-center gap-2 text-sm w-full text-left hover:bg-muted/50 rounded-md p-1.5 -ml-1.5 transition-colors duration-700 cursor-pointer ${highlight ? "bg-primary/10" : ""}`}
			>
				{isExpanded ? (
					<ChevronUp className="w-3 h-3 text-muted-foreground shrink-0" />
				) : (
					<ChevronDown className="w-3 h-3 text-muted-foreground shrink-0" />
				)}
				<span className="font-medium truncate">{step.name}</span>
				<Badge
					variant="outline"
					className={`text-[10px] font-medium shrink-0 ${getKindColor(step.kind)}`}
				>
					{step.kind}
				</Badge>
				<StatusBadge status={step.status} />
				<span className="text-xs text-muted-foreground ml-auto shrink-0">
					{formatDuration(step.duration_ms)}
				</span>
				<span className="text-xs text-muted-foreground shrink-0">
					{formatCost(step.cost_usd)}
				</span>
			</button>
			{isExpanded && (
				<div className="pl-5 space-y-3 min-w-0 overflow-hidden">
					{step.error && (
						<div className="p-2 rounded-md bg-red-50 border border-red-200">
							<div className="text-xs text-red-700 font-mono whitespace-pre-wrap break-words">
								{step.error}
							</div>
						</div>
					)}
					{step.output && <StepOutput step={step} />}
					{step.input && <StepInput step={step} />}
				</div>
			)}
		</div>
	);
}

function StepOutput({ step }: { step: StepResponse }) {
	const output = step.output;
	if (!output) return null;

	if (step.kind === "agent") {
		const value =
			typeof output.value === "string"
				? output.value
				: JSON.stringify(output.value, null, 2);
		const model = typeof output.model === "string" ? output.model : null;
		const hasContent = value.trim().length > 0;

		return (
			<div>
				<div className="flex items-center gap-2 mb-1.5">
					<span className="text-xs font-semibold text-muted-foreground">
						Agent response
					</span>
					{model && (
						<Badge
							variant="outline"
							className="text-[10px] px-1.5 py-0 bg-violet-50 text-violet-600 border-violet-200"
						>
							{model}
						</Badge>
					)}
				</div>
				<div className="max-h-96 overflow-auto border rounded-md p-4 bg-muted/50">
					{hasContent ? (
						<MarkdownContent content={value} />
					) : (
						<span className="text-sm text-muted-foreground italic">
							No response returned by the agent
						</span>
					)}
				</div>
			</div>
		);
	}

	if (step.kind === "shell") {
		const stdout = typeof output.stdout === "string" ? output.stdout : null;
		const stderr = typeof output.stderr === "string" ? output.stderr : null;
		const exitCode = output.exit_code;

		return (
			<div className="space-y-3">
				{stdout && (
					<div>
						<div className="flex items-center gap-2 mb-1.5">
							<span className="text-xs font-semibold text-muted-foreground">
								stdout
							</span>
							{exitCode !== undefined && (
								<span
									className={`text-[10px] font-mono ${exitCode === 0 ? "text-emerald-600" : "text-red-600"}`}
								>
									exit {String(exitCode)}
								</span>
							)}
						</div>
						<pre className="text-xs max-h-64 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
							{stdout}
						</pre>
					</div>
				)}
				{stderr && stderr.trim().length > 0 && (
					<div>
						<div className="text-xs font-semibold text-red-600 mb-1.5">
							stderr
						</div>
						<pre className="text-xs max-h-40 overflow-auto whitespace-pre-wrap break-words text-red-700 bg-red-50 border border-red-200 rounded-md p-3 font-mono leading-relaxed">
							{stderr}
						</pre>
					</div>
				)}
			</div>
		);
	}

	if (step.kind === "http") {
		const httpStatus = output.status;
		const body =
			typeof output.body === "string"
				? output.body
				: JSON.stringify(output.body, null, 2);

		return (
			<div>
				<div className="flex items-center gap-2 mb-1.5">
					<span className="text-xs font-semibold text-muted-foreground">
						Response
					</span>
					{httpStatus !== undefined && (
						<span
							className={`text-[10px] font-mono ${Number(httpStatus) < 400 ? "text-emerald-600" : "text-red-600"}`}
						>
							HTTP {String(httpStatus)}
						</span>
					)}
				</div>
				<pre className="text-xs max-h-64 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
					{body}
				</pre>
			</div>
		);
	}

	if (step.kind === "workflow") {
		const childRunId = typeof output.run_id === "string" ? output.run_id : null;
		const childStatus =
			typeof output.status === "string" ? (output.status as RunStatus) : null;
		const workflowName =
			typeof output.workflow_name === "string" ? output.workflow_name : null;

		return (
			<div className="space-y-3">
				<div className="flex items-center gap-3">
					<span className="text-xs font-semibold text-muted-foreground">
						Sub-workflow
					</span>
					{workflowName && (
						<span className="text-sm font-medium">{workflowName}</span>
					)}
					{childStatus && <StatusBadge status={childStatus} />}
					{childRunId && (
						<Link
							to={`/runs/${childRunId}`}
							className="text-indigo-600 hover:underline font-mono text-xs"
							onClick={(e) => e.stopPropagation()}
						>
							{childRunId}
						</Link>
					)}
				</div>
				{childRunId && <ChildRunSteps runId={childRunId} />}
			</div>
		);
	}

	return (
		<div>
			<div className="text-xs font-semibold text-muted-foreground mb-1.5">
				Output
			</div>
			<pre className="text-xs max-h-80 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
				{JSON.stringify(output, null, 2)}
			</pre>
		</div>
	);
}

function CollapsibleBlock({
	label,
	children,
	defaultOpen = false,
}: {
	label: string;
	children: React.ReactNode;
	defaultOpen?: boolean;
}) {
	return (
		<Collapsible defaultOpen={defaultOpen}>
			<CollapsibleTrigger className="flex items-center gap-1 text-xs font-semibold text-muted-foreground mb-1.5 hover:text-foreground transition-colors cursor-pointer">
				<ChevronDown className="w-3 h-3 transition-transform [[data-panel-open]_&]:rotate-180" />
				{label}
			</CollapsibleTrigger>
			<CollapsibleContent>{children}</CollapsibleContent>
		</Collapsible>
	);
}

function AgentInput({ input }: { input: Record<string, unknown> }) {
	const prompt = typeof input.prompt === "string" ? input.prompt : null;
	const systemPrompt =
		typeof input.system_prompt === "string" ? input.system_prompt : null;
	const model = typeof input.model === "string" ? input.model : null;
	const maxBudget =
		typeof input.max_budget_usd === "number" ? input.max_budget_usd : null;
	const maxTurns = typeof input.max_turns === "number" ? input.max_turns : null;
	const allowedTools = Array.isArray(input.allowed_tools)
		? (input.allowed_tools as string[])
		: null;
	const workingDir =
		typeof input.working_dir === "string" ? input.working_dir : null;
	const permissionMode =
		typeof input.permission_mode === "string" ? input.permission_mode : null;

	const configItems = [
		model && ["Model", model],
		maxBudget !== null && ["Max budget", `$${maxBudget}`],
		maxTurns !== null && ["Max turns", String(maxTurns)],
		workingDir && ["Working dir", workingDir],
		permissionMode && ["Permission", permissionMode],
		allowedTools &&
			allowedTools.length > 0 && ["Tools", allowedTools.join(", ")],
	].filter(Boolean) as [string, string][];

	return (
		<div className="space-y-3">
			<div className="text-xs font-semibold text-muted-foreground">Input</div>
			{configItems.length > 0 && (
				<div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
					{configItems.map(([label, val]) => (
						<span key={label}>
							<span className="font-semibold">{label}:</span> {val}
						</span>
					))}
				</div>
			)}
			{systemPrompt && (
				<CollapsibleBlock label="System prompt">
					<pre className="text-xs max-h-40 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
						{systemPrompt}
					</pre>
				</CollapsibleBlock>
			)}
			{prompt && (
				<CollapsibleBlock label="Prompt">
					<pre className="text-xs max-h-80 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
						{prompt}
					</pre>
				</CollapsibleBlock>
			)}
		</div>
	);
}

function StepInput({ step }: { step: StepResponse }) {
	const input = step.input;
	if (!input) return null;

	if (step.kind === "agent") {
		return <AgentInput input={input} />;
	}

	if (step.kind === "workflow") {
		return null;
	}

	if (step.kind === "shell") {
		const command = typeof input.command === "string" ? input.command : null;
		if (command) {
			return (
				<div>
					<div className="text-xs font-semibold text-muted-foreground mb-1.5">
						Command
					</div>
					<pre className="text-xs whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
						{command}
					</pre>
				</div>
			);
		}
	}

	if (step.kind === "http") {
		const method = typeof input.method === "string" ? input.method : null;
		const url = typeof input.url === "string" ? input.url : null;
		if (method && url) {
			return (
				<div>
					<div className="text-xs font-semibold text-muted-foreground mb-1.5">
						Request
					</div>
					<pre className="text-xs whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
						{method} {url}
					</pre>
				</div>
			);
		}
	}

	return (
		<div>
			<div className="text-xs font-semibold text-muted-foreground mb-1.5">
				Input
			</div>
			<pre className="text-xs max-h-80 overflow-auto whitespace-pre-wrap break-words text-foreground bg-muted/50 border rounded-md p-3 font-mono leading-relaxed">
				{JSON.stringify(input, null, 2)}
			</pre>
		</div>
	);
}

function StepRow({ step }: { step: StepResponse }) {
	const [isExpanded, setIsExpanded] = useState(false);
	const [highlight, setHighlight] = useState(false);

	useEffect(() => {
		const handleDirect = (e: Event) => {
			if ((e as CustomEvent).detail === step.id) {
				setIsExpanded(true);
				setHighlight(true);
				setTimeout(() => setHighlight(false), 1500);
			}
		};
		const handleNested = () => {
			if (step.kind === "workflow") {
				setIsExpanded(true);
			}
		};
		window.addEventListener("focus-step", handleDirect);
		window.addEventListener("focus-step-nested", handleNested);
		return () => {
			window.removeEventListener("focus-step", handleDirect);
			window.removeEventListener("focus-step-nested", handleNested);
		};
	}, [step.id, step.kind]);

	return (
		<>
			<TableRow
				id={`step-${step.id}`}
				onClick={() => setIsExpanded(!isExpanded)}
				className={`cursor-pointer hover:bg-muted/50 transition-colors duration-700 ${highlight ? "bg-primary/10" : ""}`}
			>
				<TableCell className="font-medium">{step.name}</TableCell>
				<TableCell>
					<Badge
						variant="outline"
						className={`text-xs font-medium ${getKindColor(step.kind)}`}
					>
						{step.kind}
					</Badge>
				</TableCell>
				<TableCell>
					<StatusBadge status={step.status} />
				</TableCell>
				<TableCell>{formatDuration(step.duration_ms)}</TableCell>
				<TableCell>{formatCost(step.cost_usd)}</TableCell>
				<TableCell className="text-right">
					{isExpanded ? (
						<ChevronUp className="w-4 h-4 text-muted-foreground inline" />
					) : (
						<ChevronDown className="w-4 h-4 text-muted-foreground inline" />
					)}
				</TableCell>
			</TableRow>

			{isExpanded && (
				<TableRow className="hover:bg-transparent">
					<TableCell colSpan={6} className="bg-muted/20 p-4 min-w-0">
						<div className="space-y-4 min-w-0 overflow-hidden">
							<div className="flex flex-wrap gap-4 text-xs text-muted-foreground">
								<span className="flex items-center gap-1">
									<Hash className="w-3 h-3" />
									Position {step.position}
								</span>
								<span className="flex items-center gap-1">
									<Clock className="w-3 h-3" />
									{formatDuration(step.duration_ms)}
								</span>
								<span className="flex items-center gap-1">
									<DollarSign className="w-3 h-3" />
									{formatCost(step.cost_usd)}
								</span>
								{step.input_tokens !== null && (
									<span className="flex items-center gap-1">
										<Cpu className="w-3 h-3" />
										{step.input_tokens?.toLocaleString()} in /{" "}
										{step.output_tokens?.toLocaleString()} out
									</span>
								)}
							</div>

							{step.error && (
								<div className="p-3 rounded-md bg-red-50 border border-red-200">
									<div className="text-xs font-semibold text-red-600 mb-1">
										Error
									</div>
									<div className="text-sm text-red-700 font-mono whitespace-pre-wrap break-words">
										{step.error}
									</div>
								</div>
							)}

							{step.output && <StepOutput step={step} />}

							{step.input && <StepInput step={step} />}
						</div>
					</TableCell>
				</TableRow>
			)}
		</>
	);
}

export function StepList({ steps }: StepListProps) {
	if (steps.length === 0) {
		return (
			<div className="text-center py-12 text-muted-foreground border rounded-lg bg-muted/20">
				No steps in this run
			</div>
		);
	}

	return (
		<div className="rounded-lg border overflow-hidden">
			<Table className="table-fixed">
				<TableHeader>
					<TableRow>
						<TableHead>Name</TableHead>
						<TableHead>Kind</TableHead>
						<TableHead>Status</TableHead>
						<TableHead>Duration</TableHead>
						<TableHead>Cost</TableHead>
						<TableHead className="w-10" />
					</TableRow>
				</TableHeader>
				<TableBody>
					{steps.map((step) => (
						<StepRow key={step.id} step={step} />
					))}
				</TableBody>
			</Table>
		</div>
	);
}
