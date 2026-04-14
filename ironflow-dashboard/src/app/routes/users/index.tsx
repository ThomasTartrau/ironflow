import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Plus, Shield, ShieldOff, Trash2 } from "lucide-react";
import { api } from "@/app/lib/api";
import type { UserResponse } from "@/app/lib/types";
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

	const handleDelete = (id: string, username: string) => {
		if (!confirm(`Delete user "${username}"? This cannot be undone.`)) return;
		withToast(deleteUser(id), {
			loading: "Deleting user...",
			success: "User deleted",
			error: "Failed to delete user",
		})
			.then(() => revalidator.revalidate())
			.catch(() => {});
	};

	const handleToggleRole = (id: string, currentIsAdmin: boolean) => {
		const action = currentIsAdmin ? "demote" : "promote";
		withToast(updateUserRole(id, !currentIsAdmin), {
			loading: `${action === "promote" ? "Promoting" : "Demoting"} user...`,
			success: `User ${action}d`,
			error: `Failed to ${action} user`,
		})
			.then(() => revalidator.revalidate())
			.catch(() => {});
	};

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
					<div className="text-center py-16 text-muted-foreground border rounded-lg bg-muted/20">
						No users yet. Create one to get started.
					</div>
				) : (
					<div className="rounded-lg border">
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>Username</TableHead>
									<TableHead>Email</TableHead>
									<TableHead>Role</TableHead>
									<TableHead>Created</TableHead>
									<TableHead className="w-12" />
								</TableRow>
							</TableHeader>
							<TableBody>
								{users.map((user) => (
									<TableRow key={user.id}>
										<TableCell className="font-medium">
											{user.username}
										</TableCell>
										<TableCell>{user.email}</TableCell>
										<TableCell>
											<Badge variant={user.is_admin ? "default" : "secondary"}>
												{user.is_admin ? "Admin" : "Member"}
											</Badge>
										</TableCell>
										<TableCell>{formatDate(user.created_at)}</TableCell>
										<TableCell>
											<div className="flex justify-end gap-1">
												<Button
													variant="ghost"
													size="icon-sm"
													onClick={() =>
														handleToggleRole(user.id, user.is_admin)
													}
													title={
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
												<Button
													variant="ghost"
													size="icon-sm"
													onClick={() => handleDelete(user.id, user.username)}
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
	);
}
