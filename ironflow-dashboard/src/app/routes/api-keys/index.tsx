import { useState } from "react";
import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Key, Plus, Trash2 } from "lucide-react";
import { api } from "@/app/lib/api";
import type { ApiKeyResponse, ApiKeyScope } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
import { withToast } from "@/app/lib/api-toast";
import { deleteApiKey } from "./_actions/actions";

export async function loader() {
	const res = await api.get<ApiKeyResponse[]>("/api-keys");
	return { apiKeys: res.data };
}

const SCOPE_LABELS: Record<ApiKeyScope, string> = {
	workflows_read: "Workflows Read",
	runs_read: "Runs Read",
	runs_write: "Runs Write",
	runs_manage: "Runs Manage",
	stats_read: "Stats Read",
	admin: "Admin",
};

function formatDate(iso: string): string {
	return new Date(iso).toLocaleDateString("fr-FR", {
		day: "2-digit",
		month: "short",
		year: "numeric",
	});
}

export function Component() {
	const { apiKeys } = useLoaderData() as { apiKeys: ApiKeyResponse[] };
	const navigate = useNavigate();
	const revalidator = useRevalidator();
	const [deletingId, setDeletingId] = useState<string | null>(null);
	const [confirmDeleteKey, setConfirmDeleteKey] = useState<{
		id: string;
		name: string;
	} | null>(null);

	useDocumentMeta({
		title: "API Keys",
		description: "Manage your API keys for programmatic access.",
	});

	function handleDelete(id: string, name: string) {
		setConfirmDeleteKey(null);
		setDeletingId(id);
		withToast(deleteApiKey(id), {
			loading: `Deleting ${name}...`,
			success: `${name} deleted`,
		})
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setDeletingId(null));
	}

	return (
		<HeaderApp
			title="API Keys"
			description="Manage your API keys for programmatic access."
			titleItem={
				<Button onClick={() => navigate("/api-keys/new")}>
					<Plus />
					New API Key
				</Button>
			}
		>
			<div className="space-y-6">
				{apiKeys.length === 0 ? (
					<div className="flex flex-col items-center justify-center gap-4 py-16 text-center border border-dashed rounded-[var(--radius)] bg-muted/20">
						<Key
							className="size-10 text-muted-foreground/40"
							aria-hidden="true"
						/>
						<div className="space-y-1">
							<p className="text-sm font-medium text-foreground">
								No API keys yet
							</p>
							<p className="text-xs text-muted-foreground">
								Create an API key to access Ironflow programmatically.
							</p>
						</div>
						<Button onClick={() => navigate("/api-keys/new")}>
							<Plus />
							New API Key
						</Button>
					</div>
				) : (
					<div className="rounded-[var(--radius)] border">
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>Name</TableHead>
									<TableHead>Prefix</TableHead>
									<TableHead>Scopes</TableHead>
									<TableHead>Status</TableHead>
									<TableHead>Last Used</TableHead>
									<TableHead>Created</TableHead>
									<TableHead className="w-12" />
								</TableRow>
							</TableHeader>
							<TableBody>
								{apiKeys.map((key) => (
									<TableRow key={key.id}>
										<TableCell className="font-medium">{key.name}</TableCell>
										<TableCell>
											<code className="font-mono text-xs bg-muted px-1.5 py-0.5 rounded-[var(--radius-sm)]">
												{key.key_prefix}...
											</code>
										</TableCell>
										<TableCell className="align-top">
											<div className="flex flex-wrap gap-1 max-w-[240px]">
												{key.scopes.map((scope) => (
													<Badge
														key={scope}
														variant="secondary"
														className="text-xs"
													>
														{SCOPE_LABELS[scope] ?? scope}
													</Badge>
												))}
											</div>
										</TableCell>
										<TableCell>
											{key.is_active ? (
												<Badge
													variant="outline"
													className="bg-success/10 text-success border-success/20 text-xs"
												>
													Active
												</Badge>
											) : (
												<Badge variant="secondary" className="text-xs">
													Disabled
												</Badge>
											)}
										</TableCell>
										<TableCell className="text-xs text-muted-foreground tabular-nums">
											{key.last_used_at ? (
												formatDate(key.last_used_at)
											) : (
												<span className="italic text-muted-foreground/60">
													Never
												</span>
											)}
										</TableCell>
										<TableCell className="text-xs text-muted-foreground tabular-nums">
											{formatDate(key.created_at)}
										</TableCell>
										<TableCell>
											<Button
												variant="ghost"
												size="icon-sm"
												aria-label={`Delete ${key.name}`}
												disabled={deletingId === key.id}
												onClick={() =>
													setConfirmDeleteKey({ id: key.id, name: key.name })
												}
											>
												<Trash2 className="text-destructive" />
											</Button>
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					</div>
				)}
			</div>

			<Dialog
				open={confirmDeleteKey !== null}
				onOpenChange={(open) => {
					if (!open) setConfirmDeleteKey(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete API key</DialogTitle>
						<DialogDescription>
							Are you sure you want to delete{" "}
							<span className="font-mono font-medium text-foreground">
								{confirmDeleteKey?.name}
							</span>
							? Any integrations using this key will immediately lose access.
							This action cannot be undone.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setConfirmDeleteKey(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							disabled={
								confirmDeleteKey !== null && deletingId === confirmDeleteKey.id
							}
							onClick={() => {
								if (confirmDeleteKey) {
									handleDelete(confirmDeleteKey.id, confirmDeleteKey.name);
								}
							}}
						>
							Delete
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</HeaderApp>
	);
}
