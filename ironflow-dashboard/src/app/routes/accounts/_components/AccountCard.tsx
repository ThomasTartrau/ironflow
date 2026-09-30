import { useEffect, useState } from "react";
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
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { getAccountUsage } from "../_actions/actions";
import {
	formatCountdown,
	formatPercent,
	isStale,
	stateLabel,
	stateTone,
	type Tone,
	windowTone,
} from "./account-state";

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

function Sparkline({ points }: { points: number[] }) {
	if (points.length < 2) return null;
	const width = 120;
	const height = 24;
	const step = width / (points.length - 1);
	const path = points
		.map(
			(p, i) => `${(i * step).toFixed(1)},${(height - p * height).toFixed(1)}`,
		)
		.join(" ");
	return (
		<svg
			width={width}
			height={height}
			role="img"
			aria-label="30-day utilization"
			className="text-primary"
		>
			<polyline
				points={path}
				fill="none"
				stroke="currentColor"
				strokeWidth={1.5}
			/>
		</svg>
	);
}

export function AccountCard({
	account,
	onUpdate,
	onTest,
	onDelete,
}: AccountCardProps) {
	const now = new Date(useLiveClock({ enabled: true, intervalMs: 30_000 }));
	const [history, setHistory] = useState<number[]>([]);
	const [editing, setEditing] = useState(false);
	const [confirmDelete, setConfirmDelete] = useState(false);
	const [displayName, setDisplayName] = useState(account.display_name);
	const [tags, setTags] = useState(account.tags.join(", "));
	const [priority, setPriority] = useState(String(account.priority));
	const [maxConcurrency, setMaxConcurrency] = useState(
		account.max_concurrency == null ? "" : String(account.max_concurrency),
	);
	const [threshold, setThreshold] = useState(String(account.alert_threshold));
	const [token, setToken] = useState("");

	useEffect(() => {
		let cancelled = false;
		getAccountUsage(account.id)
			.then((usage) => {
				if (cancelled) return;
				// Max utilization per observation instant.
				const byInstant = new Map<string, number>();
				for (const point of usage.history) {
					const current = byInstant.get(point.observed_at) ?? 0;
					byInstant.set(
						point.observed_at,
						Math.max(current, point.utilization),
					);
				}
				setHistory([...byInstant.values()]);
			})
			.catch(() => {
				if (!cancelled) setHistory([]);
			});
		return () => {
			cancelled = true;
		};
	}, [account.id]);

	const tone = stateTone(account.state);

	function save() {
		const edit: AccountEdit = {
			display_name: displayName,
			tags: tags
				.split(",")
				.map((t) => t.trim())
				.filter(Boolean),
			priority: Number(priority),
			max_concurrency: maxConcurrency === "" ? null : Number(maxConcurrency),
			alert_threshold: Number(threshold),
		};
		if (token !== "") edit.token = token;
		onUpdate(edit);
		setToken("");
		setEditing(false);
	}

	return (
		<Card data-testid="account-card">
			<CardContent className="space-y-3 pt-4">
				<div className="flex items-start justify-between gap-2">
					<div>
						<div className="font-semibold">{account.display_name}</div>
						<div className="text-xs text-muted-foreground">
							{account.name} - {account.kind}
							{account.plan ? ` - ${account.plan}` : ""}
						</div>
					</div>
					<span
						data-testid="account-state"
						className={`rounded-full px-2 py-0.5 text-xs font-medium ${PILL_CLASSES[tone]}`}
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
				<Sparkline points={history} />
				<div className="flex items-center justify-between">
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
					<div className="flex gap-1">
						<Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
							<Pencil className="h-4 w-4" />
						</Button>
						<Button size="sm" variant="ghost" onClick={onTest}>
							<FlaskConical className="h-4 w-4" />
						</Button>
						<Button
							size="sm"
							variant="ghost"
							onClick={() => setConfirmDelete(true)}
						>
							<Trash2 className="h-4 w-4" />
						</Button>
					</div>
				</div>
			</CardContent>

			<Dialog open={editing} onOpenChange={setEditing}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Edit {account.name}</DialogTitle>
						<DialogDescription>
							The token is write-only: leave it empty to keep the current one.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-2">
						<Input
							aria-label="Display name"
							value={displayName}
							onChange={(e) => setDisplayName(e.target.value)}
						/>
						<Input
							aria-label="Tags"
							placeholder="tags, comma separated"
							value={tags}
							onChange={(e) => setTags(e.target.value)}
						/>
						<Input
							aria-label="Priority"
							type="number"
							value={priority}
							onChange={(e) => setPriority(e.target.value)}
						/>
						<Input
							aria-label="Max concurrency"
							type="number"
							min={1}
							placeholder="unlimited"
							value={maxConcurrency}
							onChange={(e) => setMaxConcurrency(e.target.value)}
						/>
						<Input
							aria-label="Alert threshold"
							type="number"
							step="0.05"
							min={0.05}
							max={1}
							value={threshold}
							onChange={(e) => setThreshold(e.target.value)}
						/>
						<Input
							aria-label="New token"
							type="password"
							autoComplete="off"
							placeholder="sk-ant-oat01-..."
							value={token}
							onChange={(e) => setToken(e.target.value)}
						/>
					</div>
					<DialogFooter>
						<Button variant="outline" onClick={() => setEditing(false)}>
							Cancel
						</Button>
						<Button onClick={save}>Save</Button>
					</DialogFooter>
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
