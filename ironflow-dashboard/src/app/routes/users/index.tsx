import { useState } from "react";
import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Plus, Shield, ShieldOff, Trash2, Users } from "lucide-react";
import { api } from "@/app/lib/api";
import type { UserResponse } from "@/app/lib/types";
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
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { withToast } from "@/app/lib/api-toast";
import { deleteUser, updateUserRole } from "./_actions/actions";

export async function loader() {
	const res = await api.get<UserResponse[]>("/users?page=1&per_page=100");
	return { users: res.data };
}

function formatDate(iso: string): string {
	return new Date(iso).toLocaleDateString("fr-FR", {
		day: "2-digit",
		month: "short",
		year: "numeric",
	});
}

export function Component() {
	useDocumentMeta({ title: "Users" });
	const { users } = useLoaderData<typeof loader>();
	const navigate = useNavigate();
	const revalidator = useRevalidator();
	const auth = useAppSelector((state) => state.auth);
	const currentUserId =
		auth.status === "authenticated" ? auth.user.user_id : null;

	const [confirmDelete, setConfirmDelete] = useState<{
		id: string;
		username: string;
	} | null>(null);
	const [deletingId, setDeletingId] = useState<string | null>(null);

	function handleDelete(id: string, _username: string) {
		setConfirmDelete(null);
		setDeletingId(id);
		withToast(deleteUser(id), {
			loading: "Deleting user...",
			success: "User deleted",
			error: "Failed to delete user",
		})
			.then(() => revalidator.revalidate())
			.catch(() => {})
			.finally(() => setDeletingId(null));
	}

	function handleToggleRole(id: string, currentIsAdmin: boolean) {
		const action = currentIsAdmin ? "demote" : "promote";
		withToast(updateUserRole(id, !currentIsAdmin), {
			loading: `${action === "promote" ? "Promoting" : "Demoting"} user...`,
			success: `User ${action}d`,
			error: `Failed to ${action} user`,
		})
			.then(() => revalidator.revalidate())
			.catch(() => {});
	}

	const isSelfDelete =
		confirmDelete !== null && confirmDelete.id === currentUserId;

	return (
		<HeaderApp
			title="Users"
			description="Manage user accounts and roles."
			titleItem={
				<Button onClick={() => navigate("/users/new")}>
					<Plus className="h-4 w-4 mr-1" />
					New User
				</Button>
			}
		>
			<div className="space-y-6">
				{users.length === 0 ? (
					<div className="flex flex-col items-center justify-center py-20 gap-4 border rounded-[var(--radius-xl)] bg-muted/20 text-center">
						<Users
							className="size-8 text-muted-foreground/40"
							aria-hidden="true"
						/>
						<div>
							<p className="text-sm font-medium text-foreground">
								No users yet
							</p>
							<p className="text-xs text-muted-foreground mt-1">
								Create the first user account to get started.
							</p>
						</div>
						<Button size="sm" onClick={() => navigate("/users/new")}>
							<Plus className="h-4 w-4 mr-1" />
							New User
						</Button>
					</div>
				) : (
					<div className="rounded-[var(--radius-xl)] border">
						<Table aria-label="User accounts">
							<TableHeader>
								<TableRow>
									<TableHead>Username</TableHead>
									<TableHead>Email</TableHead>
									<TableHead>Role</TableHead>
									<TableHead>Created</TableHead>
									<TableHead className="w-20" />
								</TableRow>
							</TableHeader>
							<TableBody>
								{users.map((user) => (
									<TableRow key={user.id}>
										<TableCell className="font-semibold text-foreground font-mono text-xs">
											{user.username}
										</TableCell>
										<TableCell className="text-muted-foreground text-xs">
											{user.email}
										</TableCell>
										<TableCell>
											{user.is_admin ? (
												<Badge
													variant="outline"
													className="border-primary/40 text-primary rounded-[var(--radius-sm)] text-[11px] px-1.5 py-0.5 font-medium uppercase tracking-wide"
												>
													Admin
												</Badge>
											) : (
												<Badge
													variant="secondary"
													className="rounded-[var(--radius-sm)] text-[11px] px-1.5 py-0.5 font-medium uppercase tracking-wide"
												>
													Member
												</Badge>
											)}
										</TableCell>
										<TableCell className="tabular-nums text-xs text-muted-foreground">
											{formatDate(user.created_at)}
										</TableCell>
										<TableCell>
											<div className="flex justify-end gap-1">
												<TooltipProvider>
													<Tooltip>
														<TooltipTrigger
															render={
																<Button
																	variant="ghost"
																	size="icon-sm"
																	onClick={() =>
																		handleToggleRole(user.id, user.is_admin)
																	}
																	aria-label={
																		user.is_admin
																			? "Demote to member"
																			: "Promote to admin"
																	}
																>
																	{user.is_admin ? (
																		<ShieldOff className="h-4 w-4" />
																	) : (
																		<Shield className="h-4 w-4" />
																	)}
																</Button>
															}
														/>
														<TooltipContent>
															{user.is_admin
																? "Demote to member"
																: "Promote to admin"}
														</TooltipContent>
													</Tooltip>
												</TooltipProvider>
												<Button
													variant="ghost"
													size="icon-sm"
													className="text-destructive hover:text-destructive hover:bg-destructive/10"
													onClick={() =>
														setConfirmDelete({
															id: user.id,
															username: user.username,
														})
													}
													aria-label={`Delete user ${user.username}`}
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

			<Dialog
				open={confirmDelete !== null}
				onOpenChange={(open) => {
					if (!open) setConfirmDelete(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete user</DialogTitle>
						<DialogDescription>
							Are you sure you want to delete{" "}
							<span className="font-mono font-medium text-foreground">
								{confirmDelete?.username}
							</span>
							?
							{isSelfDelete && (
								<>
									{" "}
									<span className="text-destructive font-medium">
										You are about to delete your own account.
									</span>
								</>
							)}{" "}
							This action cannot be undone.
						</DialogDescription>
					</DialogHeader>
					<DialogFooter>
						<Button variant="outline" onClick={() => setConfirmDelete(null)}>
							Cancel
						</Button>
						<Button
							variant="destructive"
							disabled={
								confirmDelete !== null && deletingId === confirmDelete.id
							}
							onClick={() => {
								if (confirmDelete) {
									handleDelete(confirmDelete.id, confirmDelete.username);
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
