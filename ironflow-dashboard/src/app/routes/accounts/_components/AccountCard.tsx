import { useState } from "react";
import { FlaskConical, Pencil, Trash2 } from "lucide-react";
import type {
	AccountWindowResponse,
	ProviderAccountResponse,
} from "@/app/lib/types";
import { useLiveClock } from "@/app/hooks/use-live-clock";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	formatCountdown,
	formatPercent,
	isStale,
	kindLabel,
	planLabel,
	stateLabel,
	stateTone,
	type Tone,
	windowTone,
} from "./account-state";
import {
	AccountFormFields,
	accountFormValues,
	parseAccountForm,
} from "./AccountFormFields";

const TONE_CLASSES: Record<Tone, string> = {
	success: "bg-emerald-500",
	warning: "bg-amber-500",
	danger: "bg-red-500",
	muted: "bg-muted-foreground",
};

const PILL_CLASSES: Record<Tone, string> = {
	success: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400",
	warning: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
	danger: "bg-red-500/15 text-red-700 dark:text-red-400",
	muted: "bg-muted text-muted-foreground",
};

const WINDOW_LABELS: Record<string, string> = {
	five_hour: "5 hours",
	seven_day: "7 days",
};

export interface AccountEdit {
	display_name?: string;
	tags?: string[];
	priority?: number;
	max_concurrency?: number | null;
	alert_threshold?: number;
	token?: string;
	enabled?: boolean;
}

interface AccountCardProps {
	account: ProviderAccountResponse;
	onUpdate: (edit: AccountEdit) => void;
	onTest: () => void;
	onDelete: () => void;
}

function timeAgo(observedAt: string, now: Date): string {
	const minutes = Math.max(
		0,
		Math.floor((now.getTime() - new Date(observedAt).getTime()) / 60_000),
	);
	if (minutes === 0) return "now";
	if (minutes < 60) return `${minutes}m ago`;
	const hours = Math.floor(minutes / 60);
	if (hours < 48) return `${hours}h ago`;
	return `${Math.floor(hours / 24)}d ago`;
}

function Gauge({ window, now }: { window: AccountWindowResponse; now: Date }) {
	const label =
		(WINDOW_LABELS[window.window] ?? window.window) +
		(window.model_scope ? ` (${window.model_scope})` : "");
	const countdown = formatCountdown(window.resets_at, now);
	const stale = isStale(window.observed_at, now);
	return (
		<div data-testid="account-gauge" className="space-y-1">
			<div className="flex justify-between text-xs">
				<span className="font-medium">{label}</span>
				<span>{formatPercent(window.utilization)}</span>
			</div>
			<div className="h-2 w-full rounded bg-muted">
				<div
					className={`h-2 rounded ${TONE_CLASSES[windowTone(window.status)]}`}
					style={{ width: formatPercent(window.utilization) }}
				/>
			</div>
			<div className="flex justify-between text-[11px] text-muted-foreground">
				<span>{countdown ? `resets in ${countdown}` : ""}</span>
				<span className={stale ? "opacity-50" : ""} data-stale={stale}>
					observed {timeAgo(window.observed_at, now)}
				</span>
			</div>
		</div>
	);
}

function CardAction({
	label,
	onClick,
	destructive,
	children,
}: {
	label: string;
	onClick: () => void;
	destructive?: boolean;
	children: React.ReactNode;
}) {
	return (
		<Tooltip>
			<TooltipTrigger
				render={
					<Button
						size="icon-sm"
						variant="ghost"
						aria-label={label}
						onClick={onClick}
						className={destructive ? "text-destructive" : undefined}
					>
						{children}
					</Button>
				}
			/>
			<TooltipContent side="bottom">
				<span className="text-xs">{label}</span>
			</TooltipContent>
		</Tooltip>
	);
}

export function AccountCard({
	account,
	onUpdate,
	onTest,
	onDelete,
}: AccountCardProps) {
	const now = new Date(useLiveClock({ enabled: true, intervalMs: 30_000 }));
	const [editing, setEditing] = useState(false);
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [values, setValues] = useState(() => accountFormValues(account));

	const tone = stateTone(account.state);

	function openEdit() {
		setValues(accountFormValues(account));
		setEditing(true);
	}

	function save(event: React.FormEvent) {
		event.preventDefault();
		const edit: AccountEdit = {
			...parseAccountForm(values),
			display_name: values.displayName,
		};
		if (values.token !== "") edit.token = values.token;
		onUpdate(edit);
		setEditing(false);
	}

	return (
		<Card data-testid="account-card" className="h-full">
			<CardContent className="flex flex-1 flex-col gap-3 pt-4">
				<div className="flex items-start justify-between gap-2">
					<div className="min-w-0">
						<div className="font-semibold">{account.display_name}</div>
						<div className="text-xs text-muted-foreground">
							<span className="font-mono">{account.name}</span>
							{" - "}
							{kindLabel(account.kind)}
							{account.plan ? `, ${planLabel(account.plan)}` : ""}
						</div>
					</div>
					<span
						data-testid="account-state"
						className={`shrink-0 whitespace-nowrap rounded-full px-2 py-0.5 text-xs font-medium ${PILL_CLASSES[tone]}`}
					>
						{stateLabel(account.state)}
					</span>
				</div>
				<div className="flex flex-wrap gap-1">
					{account.tags.map((tag) => (
						<Badge key={tag} variant="outline">
							{tag}
						</Badge>
					))}
				</div>
				<div className="space-y-2">
					{account.windows.length === 0 ? (
						<div className="text-xs text-muted-foreground">
							No usage observed yet.
						</div>
					) : (
						account.windows.map((w) => (
							<Gauge
								key={`${w.window}-${w.model_scope ?? ""}`}
								window={w}
								now={now}
							/>
						))
					)}
				</div>
				<div className="mt-auto flex items-center justify-between pt-2">
					<label
						htmlFor={`account-enabled-${account.id}`}
						className="flex items-center gap-2 text-xs"
					>
						<Switch
							id={`account-enabled-${account.id}`}
							checked={account.enabled}
							onCheckedChange={(checked: boolean) =>
								onUpdate({ enabled: checked })
							}
							aria-label="Enabled"
						/>
						Enabled
					</label>
					<TooltipProvider delay={200}>
						<div className="flex gap-1">
							<CardAction label="Edit account" onClick={openEdit}>
								<Pencil />
							</CardAction>
							<CardAction label="Test the token now" onClick={onTest}>
								<FlaskConical />
							</CardAction>
							<CardAction
								label="Delete account"
								destructive
								onClick={() => setConfirmDelete(true)}
							>
								<Trash2 />
							</CardAction>
						</div>
					</TooltipProvider>
				</div>
			</CardContent>

			<Dialog open={editing} onOpenChange={setEditing}>
				<DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
					<DialogHeader>
						<DialogTitle>Edit {account.display_name}</DialogTitle>
						<DialogDescription>
							<span className="font-mono">{account.name}</span> -{" "}
							{kindLabel(account.kind)}. The identifier and the kind cannot be
							changed.
						</DialogDescription>
					</DialogHeader>
					<form className="space-y-4" onSubmit={save}>
						<AccountFormFields
							mode="edit"
							idPrefix={`account-${account.id}`}
							values={values}
							onChange={setValues}
						/>
						<DialogFooter>
							<Button
								type="button"
								variant="outline"
								onClick={() => setEditing(false)}
							>
								Cancel
							</Button>
							<Button type="submit">Save</Button>
						</DialogFooter>
					</form>
				</DialogContent>
			</Dialog>

			<Dialog open={confirmDelete} onOpenChange={setConfirmDelete}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete {account.name}?</DialogTitle>
						<DialogDescription>
							The account, its token and its usage history are deleted.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setConfirmDelete(false)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={() => {
								setConfirmDelete(false);
								onDelete();
							}}
						>
							Delete
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</Card>
	);
}
