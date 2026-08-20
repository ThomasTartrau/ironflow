import { useState } from "react";
import { Link } from "react-router";
import type { AuditLogEntry, UserResponse } from "@/app/lib/types";
import { capitalize } from "@/app/lib/format";
import { Eye, ScrollText } from "lucide-react";
import { Button } from "@/components/ui/button";
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
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { TimeAgo } from "@/app/components/TimeAgo";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";

interface AuditLogsTableProps {
	entries: AuditLogEntry[];
	userMap?: Map<string, UserResponse>;
}

export function AuditLogsTable({ entries, userMap }: AuditLogsTableProps) {
	const [payloadEntry, setPayloadEntry] = useState<AuditLogEntry | null>(null);

	if (entries.length === 0) {
		return (
			<div className="flex flex-col items-center gap-3 py-16 border border-dashed rounded-[var(--radius-xl)] bg-muted/20 text-center">
				<ScrollText className="size-8 text-muted-foreground/40" />
				<p className="text-sm font-medium text-foreground">
					No audit logs found
				</p>
				<p className="text-xs text-muted-foreground">
					Events will appear here as they occur.
				</p>
			</div>
		);
	}

	return (
		<>
			<div className="rounded-[var(--radius-xl)] border overflow-x-auto">
				<Table aria-label="Audit logs">
					<TableHeader>
						<TableRow>
							<TableHead>Date</TableHead>
							<TableHead>Event</TableHead>
							<TableHead>Run</TableHead>
							<TableHead>User</TableHead>
							<TableHead className="w-16">Payload</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{entries.map((entry) => (
							<TableRow key={entry.id}>
								<TableCell className="tabular-nums text-xs text-muted-foreground whitespace-nowrap">
									<TimeAgo date={entry.created_at} />
								</TableCell>
								<TableCell className="font-mono text-xs font-medium whitespace-nowrap">
									{capitalize(entry.event_type)}
								</TableCell>
								<TableCell className="font-mono text-xs">
									{entry.run_id ? (
										<Link
											to={`/runs/${entry.run_id}`}
											className="text-primary hover:underline"
										>
											{entry.run_id.slice(0, 8)}
										</Link>
									) : (
										<span className="text-muted-foreground">-</span>
									)}
								</TableCell>
								<TableCell className="text-xs">
									<UserCell userId={entry.user_id} userMap={userMap} />
								</TableCell>
								<TableCell>
									<Button
										variant="ghost"
										size="icon-sm"
										aria-label="View payload"
										onClick={() => setPayloadEntry(entry)}
									>
										<Eye className="h-4 w-4" />
									</Button>
								</TableCell>
							</TableRow>
						))}
					</TableBody>
				</Table>
			</div>

			<Dialog
				open={payloadEntry !== null}
				onOpenChange={(open) => {
					if (!open) setPayloadEntry(null);
				}}
			>
				<DialogContent className="max-w-lg">
					<DialogHeader>
						<DialogTitle>
							{payloadEntry ? capitalize(payloadEntry.event_type) : "Payload"}
						</DialogTitle>
						<DialogDescription>
							{payloadEntry && <TimeAgo date={payloadEntry.created_at} />}
						</DialogDescription>
					</DialogHeader>
					<div className="overflow-x-auto rounded-[var(--radius-md)] bg-muted p-3">
						<pre className="text-xs font-mono whitespace-pre-wrap break-all">
							{payloadEntry
								? JSON.stringify(payloadEntry.payload, null, 2)
								: ""}
						</pre>
					</div>
				</DialogContent>
			</Dialog>
		</>
	);
}

function UserCell({
	userId,
	userMap,
}: {
	userId: string | null | undefined;
	userMap?: Map<string, UserResponse>;
}) {
	if (!userId) {
		return <span className="text-muted-foreground">-</span>;
	}

	const user = userMap?.get(userId);

	if (user) {
		return (
			<TooltipProvider>
				<Tooltip>
					<TooltipTrigger
						render={
							<span className="font-mono font-medium text-foreground cursor-default">
								{user.username}
							</span>
						}
					/>
					<TooltipContent>
						<p className="text-xs">{user.email}</p>
						<p className="text-xs text-muted-foreground font-mono">
							{userId.slice(0, 8)}
						</p>
					</TooltipContent>
				</Tooltip>
			</TooltipProvider>
		);
	}

	return (
		<span className="font-mono text-muted-foreground">
			{userId.slice(0, 8)}
		</span>
	);
}
