import { useState } from "react";
import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import {
	Plus,
	Trash2,
	Pencil,
	LockKeyhole,
	TriangleAlert,
	RotateCw,
	KeyRound,
} from "lucide-react";
import { api } from "@/app/lib/api";
import type {
	SecretResponse,
	KeyVersionsResponse,
	RotateSecretsResponse,
} from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useAppSelector } from "@/app/store";
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
import { TimeAgo } from "@/app/components/TimeAgo";
import { withToast } from "@/app/lib/api-toast";
import {
	deleteSecret,
	updateSecret,
	rotateSecrets,
	getKeyVersions,
} from "./_actions/actions";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
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
	const auth = useAppSelector((state) => state.auth);
	const isAdmin = auth.status === "authenticated" && auth.user.is_admin;
	const [deletingKey, setDeletingKey] = useState<string | null>(null);
	const [pendingDeleteKey, setPendingDeleteKey] = useState<string | null>(null);
	const [editingSecret, setEditingSecret] = useState<SecretResponse | null>(
		null,
	);
	const [editValue, setEditValue] = useState("");
	const [editPending, setEditPending] = useState(false);
	const [rotating, setRotating] = useState(false);
	const [versionsOpen, setVersionsOpen] = useState(false);
	const [keyVersions, setKeyVersions] = useState<KeyVersionsResponse | null>(
		null,
	);
	const [versionsLoading, setVersionsLoading] = useState(false);

	useDocumentMeta({
		title: "Secrets",
		description: "Manage encrypted secrets accessible from workflows.",
	});

	function handleDeleteRequest(key: string) {
		setPendingDeleteKey(key);
	}

	function handleDeleteConfirm() {
		if (!pendingDeleteKey) return;
		const key = pendingDeleteKey;
		setPendingDeleteKey(null);
		setDeletingKey(key);
		withToast(deleteSecret(key), {
			loading: `Deleting ${key}...`,
			success: `${key} deleted`,
		})
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setDeletingKey(null));
	}

	function handleDeleteCancel() {
		setPendingDeleteKey(null);
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

	async function handleRotate() {
		setRotating(true);
		let afterId: string | null = null;
		let totalRotated = 0;
		let totalFailed = 0;

		const doRotation = async (): Promise<RotateSecretsResponse> => {
			const body = afterId ? { after_id: afterId } : {};
			return rotateSecrets(body);
		};

		await withToast(
			(async () => {
				let batch = await doRotation();
				totalRotated += batch.rotated;
				totalFailed += batch.failed;

				while (batch.remaining > 0 && batch.last_id) {
					afterId = batch.last_id;
					batch = await doRotation();
					totalRotated += batch.rotated;
					totalFailed += batch.failed;
				}

				return { rotated: totalRotated, failed: totalFailed };
			})(),
			{
				loading: "Rotating secrets...",
				success: `Rotation complete: ${totalRotated} rotated, ${totalFailed} failed`,
			},
		).catch(() => {});

		setRotating(false);
		revalidator.revalidate();
	}

	async function handleOpenVersions() {
		setVersionsOpen(true);
		setVersionsLoading(true);
		try {
			const data = await getKeyVersions();
			setKeyVersions(data);
		} catch {
			setKeyVersions(null);
		} finally {
			setVersionsLoading(false);
		}
	}

	return (
		<>
			<HeaderApp
				title="Secrets"
				description="Encrypted secrets accessible from workflows."
				titleItem={
					<div className="flex gap-2">
						{isAdmin && (
							<>
								<Button
									variant="outline"
									onClick={handleOpenVersions}
									disabled={versionsLoading}
								>
									<KeyRound className="h-4 w-4 mr-1" />
									Key Versions
								</Button>
								<Button
									variant="outline"
									onClick={handleRotate}
									disabled={rotating}
								>
									<RotateCw
										className={`h-4 w-4 mr-1 ${rotating ? "animate-spin" : ""}`}
									/>
									{rotating ? "Rotating..." : "Rotate"}
								</Button>
							</>
						)}
						<Button onClick={() => navigate("/secrets/new")}>
							<Plus className="h-4 w-4 mr-1" />
							New Secret
						</Button>
					</div>
				}
			>
				<div className="space-y-6">
					{secrets.length === 0 ? (
						<div className="flex flex-col items-center gap-3 py-16 border border-dashed rounded-[var(--radius-xl)] bg-muted/20 text-center">
							<LockKeyhole className="size-8 text-muted-foreground/40" />
							<p className="text-sm font-medium text-foreground">
								No secrets yet
							</p>
							<p className="text-xs text-muted-foreground">
								Store API keys, tokens, and sensitive credentials.
							</p>
							<Button size="sm" onClick={() => navigate("/secrets/new")}>
								<Plus className="h-4 w-4 mr-1" />
								New Secret
							</Button>
						</div>
					) : (
						<div className="rounded-[var(--radius-xl)] border">
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Key</TableHead>
										<TableHead>Value</TableHead>
										<TableHead>Created</TableHead>
										<TableHead>Updated</TableHead>
										<TableHead className="w-16" />
									</TableRow>
								</TableHeader>
								<TableBody>
									{secrets.map((secret) => (
										<TableRow key={secret.id}>
											<TableCell className="font-mono text-sm font-semibold text-foreground">
												{secret.key}
											</TableCell>
											<TableCell>
												<span className="inline-flex items-center gap-1.5 text-muted-foreground font-mono tracking-widest text-xs">
													<LockKeyhole className="size-3 shrink-0" />
													{"••••••••"}
												</span>
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
														aria-label={`Edit secret ${secret.key}`}
														onClick={() => handleEditOpen(secret)}
														disabled={deletingKey === secret.key}
													>
														<Pencil className="h-4 w-4" />
													</Button>
													<Button
														variant="ghost"
														size="icon-sm"
														aria-label={`Delete secret ${secret.key}`}
														className="text-muted-foreground hover:text-destructive hover:bg-destructive/10"
														onClick={() => handleDeleteRequest(secret.key)}
														disabled={deletingKey === secret.key}
													>
														<Trash2 className="h-4 w-4" />
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

			{/* Delete confirmation dialog */}
			<Dialog
				open={pendingDeleteKey !== null}
				onOpenChange={(open) => {
					if (!open) handleDeleteCancel();
				}}
			>
				<DialogContent showCloseButton={false}>
					<DialogHeader>
						<DialogTitle className="flex items-center gap-2">
							<TriangleAlert
								className="size-4 text-destructive"
								aria-hidden="true"
							/>
							Delete secret
						</DialogTitle>
						<DialogDescription>
							Permanently delete{" "}
							<span className="font-mono font-semibold text-foreground">
								{pendingDeleteKey}
							</span>
							? This action cannot be undone. Any workflow referencing this
							secret will fail.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={handleDeleteCancel}
							type="button"
						>
							Cancel
						</Button>
						<Button
							variant="destructive"
							onClick={handleDeleteConfirm}
							type="button"
						>
							Delete
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Key versions dialog */}
			<Dialog
				open={versionsOpen}
				onOpenChange={(open) => {
					if (!open) {
						setVersionsOpen(false);
						setKeyVersions(null);
					}
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Key Versions</DialogTitle>
						<DialogDescription>
							Encryption key ring status for stored secrets.
						</DialogDescription>
					</DialogHeader>
					{versionsLoading ? (
						<p className="text-sm text-muted-foreground py-4">Loading...</p>
					) : keyVersions ? (
						<div className="space-y-3 text-sm">
							<div className="flex items-center gap-2">
								<span className="font-medium">Active version:</span>
								<Badge variant="outline">{keyVersions.active}</Badge>
							</div>
							<div>
								<span className="font-medium">Configured:</span>{" "}
								<span className="font-mono text-xs">
									{keyVersions.configured.join(", ")}
								</span>
							</div>
							<div>
								<span className="font-medium">In use:</span>{" "}
								<span className="font-mono text-xs">
									{keyVersions.in_use.join(", ") || "none"}
								</span>
							</div>
							<div>
								<span className="font-medium">Retirable:</span>{" "}
								<span className="font-mono text-xs">
									{keyVersions.retirable.join(", ") || "none"}
								</span>
							</div>
							{keyVersions.missing.length > 0 && (
								<div className="flex items-start gap-2 p-3 rounded-[var(--radius-md)] bg-destructive/10 text-destructive text-xs">
									<TriangleAlert className="size-4 shrink-0 mt-0.5" />
									<div>
										<p className="font-medium">Missing key versions</p>
										<p>
											Versions {keyVersions.missing.join(", ")} are used by
											stored secrets but not in the configured key ring. These
											secrets cannot be decrypted.
										</p>
									</div>
								</div>
							)}
						</div>
					) : (
						<p className="text-sm text-destructive py-4">
							Failed to load key versions.
						</p>
					)}
					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => {
								setVersionsOpen(false);
								setKeyVersions(null);
							}}
							type="button"
						>
							Close
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			{/* Edit dialog */}
			<Dialog open={editingSecret !== null} onOpenChange={handleEditClose}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Edit Secret</DialogTitle>
						<DialogDescription>
							The current value cannot be displayed. Enter a new value to
							replace it.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-4">
						<div>
							<p id="edit-key-label" className="text-sm font-medium">
								Key (read-only)
							</p>
							<div className="mt-1 px-3 py-2 bg-muted rounded-[var(--radius-md)] text-sm font-mono select-all cursor-text">
								{editingSecret?.key}
							</div>
						</div>
						<div>
							<label htmlFor="secret-value" className="text-sm font-medium">
								New value
							</label>
							<Textarea
								id="secret-value"
								placeholder="Enter the secret value"
								value={editValue}
								onChange={(e) => setEditValue(e.target.value)}
								className="mt-1 min-h-[6rem]"
							/>
						</div>
					</div>
					<DialogFooter>
						<Button
							variant="outline"
							onClick={handleEditClose}
							disabled={editPending}
							type="button"
						>
							Cancel
						</Button>
						<Button
							onClick={handleEditSave}
							disabled={!editValue.trim() || editPending}
							type="button"
							className="min-w-[5rem]"
						>
							{editPending ? "Saving..." : "Save"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
}
