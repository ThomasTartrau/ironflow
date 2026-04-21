import { useState } from "react";
import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Plus, Trash2, Pencil } from "lucide-react";
import { api } from "@/app/lib/api";
import type { SecretResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import { TimeAgo } from "@/app/components/TimeAgo";
import { withToast } from "@/app/lib/api-toast";
import { deleteSecret, updateSecret } from "./_actions/actions";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";

export async function loader() {
	const res = await api.get<SecretResponse[]>("/secrets");
	return { secrets: res.data };
}

export function Component() {
	const { secrets } = useLoaderData() as { secrets: SecretResponse[] };
	const navigate = useNavigate();
	const revalidator = useRevalidator();
	const [deletingKey, setDeletingKey] = useState<string | null>(null);
	const [editingSecret, setEditingSecret] = useState<SecretResponse | null>(
		null,
	);
	const [editValue, setEditValue] = useState("");
	const [editPending, setEditPending] = useState(false);

	useDocumentMeta({
		title: "Secrets",
		description: "Manage encrypted secrets accessible from workflows.",
	});

	function handleDelete(key: string) {
		if (!confirm(`Delete secret "${key}"? This cannot be undone.`)) return;
		setDeletingKey(key);
		withToast(deleteSecret(key), {
			loading: `Deleting ${key}...`,
			success: `${key} deleted`,
		})
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setDeletingKey(null));
	}

	function handleEditOpen(secret: SecretResponse) {
		setEditingSecret(secret);
		setEditValue("");
	}

	function handleEditClose() {
		setEditingSecret(null);
		setEditValue("");
		setEditPending(false);
	}

	function handleEditSave() {
		if (!editingSecret || !editValue.trim()) return;

		setEditPending(true);
		withToast(updateSecret(editingSecret.key, { value: editValue }), {
			loading: "Updating secret...",
			success: "Secret updated",
		})
			.then(() => {
				revalidator.revalidate();
				handleEditClose();
			})
			.catch(() => {})
			.finally(() => setEditPending(false));
	}

	return (
		<>
			<HeaderApp
				title="Secrets"
				description="Encrypted secrets accessible from workflows."
				titleItem={
					<Button onClick={() => navigate("/secrets/new")}>
						<Plus className="h-4 w-4 mr-1" />
						New Secret
					</Button>
				}
			>
				<div className="space-y-6">
					{secrets.length === 0 ? (
						<div className="text-center py-16 text-muted-foreground border rounded-lg bg-muted/20">
							No secrets yet. Create one to get started.
						</div>
					) : (
						<div className="rounded-lg border">
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Key</TableHead>
										<TableHead>Created</TableHead>
										<TableHead>Updated</TableHead>
										<TableHead className="w-16" />
									</TableRow>
								</TableHeader>
								<TableBody>
									{secrets.map((secret) => (
										<TableRow key={secret.id}>
											<TableCell className="font-medium">
												{secret.key}
											</TableCell>
											<TableCell>
												<TimeAgo date={secret.created_at} />
											</TableCell>
											<TableCell>
												<TimeAgo date={secret.updated_at} />
											</TableCell>
											<TableCell>
												<div className="flex justify-end gap-1">
													<Button
														variant="ghost"
														size="icon-sm"
														onClick={() => handleEditOpen(secret)}
													>
														<Pencil className="h-4 w-4" />
													</Button>
													<Button
														variant="ghost"
														size="icon-sm"
														onClick={() => handleDelete(secret.key)}
														disabled={deletingKey === secret.key}
													>
														<Trash2 className="h-4 w-4 text-destructive" />
													</Button>
												</div>
											</TableCell>
										</TableRow>
									))}
								</TableBody>
							</Table>
						</div>
					)}
				</div>
			</HeaderApp>

			<Dialog open={editingSecret !== null} onOpenChange={handleEditClose}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Edit Secret</DialogTitle>
						<DialogDescription>
							Update the value for {editingSecret?.key}
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-4">
						<div>
							<span className="text-sm font-medium">Key (read-only)</span>
							<div className="mt-1 px-3 py-2 bg-muted rounded text-sm">
								{editingSecret?.key}
							</div>
						</div>
						<div>
							<label htmlFor="secret-value" className="text-sm font-medium">
								Value
							</label>
							<Textarea
								id="secret-value"
								placeholder="Enter the secret value"
								value={editValue}
								onChange={(e) => setEditValue(e.target.value)}
								className="mt-1"
								rows={4}
							/>
						</div>
						<div className="flex justify-end gap-2">
							<Button
								variant="outline"
								onClick={handleEditClose}
								disabled={editPending}
							>
								Cancel
							</Button>
							<Button
								onClick={handleEditSave}
								disabled={!editValue.trim() || editPending}
							>
								{editPending ? "Saving..." : "Save"}
							</Button>
						</div>
					</div>
				</DialogContent>
			</Dialog>
		</>
	);
}
