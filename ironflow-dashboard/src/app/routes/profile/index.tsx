import { useState, type FormEvent } from "react";
import { User, Lock } from "lucide-react";
import { api } from "@/app/lib/api";
import { useAppSelector } from "@/app/store";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { HeaderApp } from "@/app/components/HeaderApp";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { withToast } from "@/app/lib/api-toast";
import { TimeAgo } from "@/app/components/TimeAgo";

export function Component() {
	const auth = useAppSelector((state) => state.auth);

	useDocumentMeta({
		title: "Profile",
		description: "View your account details and change your password.",
	});

	const [oldPassword, setOldPassword] = useState("");
	const [newPassword, setNewPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [changing, setChanging] = useState(false);

	if (auth.status !== "authenticated") {
		return null;
	}

	const { user } = auth;

	async function handleChangePassword(e: FormEvent) {
		e.preventDefault();
		if (newPassword !== confirmPassword) {
			return;
		}
		setChanging(true);
		await withToast(
			api.patch("/auth/password", {
				old_password: oldPassword,
				new_password: newPassword,
			}),
			{
				loading: "Changing password...",
				success: "Password changed",
				error: "Failed to change password",
			},
		);
		setOldPassword("");
		setNewPassword("");
		setConfirmPassword("");
		setChanging(false);
	}

	const passwordMismatch =
		confirmPassword.length > 0 && newPassword !== confirmPassword;
	const canSubmit =
		oldPassword.length > 0 &&
		newPassword.length >= 8 &&
		newPassword === confirmPassword &&
		!changing;

	return (
		<>
			<HeaderApp
				title="Profile"
				description="View your account details and change your password."
			/>

			<div className="p-6 max-w-2xl space-y-6">
				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2">
							<User className="size-5" />
							Account Details
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="grid grid-cols-[120px_1fr] gap-y-3 text-sm">
							<span className="text-muted-foreground">Username</span>
							<span className="font-medium">{user.username}</span>

							<span className="text-muted-foreground">Email</span>
							<span className="font-medium">{user.email}</span>

							<span className="text-muted-foreground">Role</span>
							<span>
								<Badge variant={user.is_admin ? "default" : "secondary"}>
									{user.is_admin ? "Admin" : "Member"}
								</Badge>
							</span>

							<span className="text-muted-foreground">Joined</span>
							<span>
								<TimeAgo date={user.created_at} />
							</span>
						</div>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2">
							<Lock className="size-5" />
							Change Password
						</CardTitle>
					</CardHeader>
					<CardContent>
						<form onSubmit={handleChangePassword} className="space-y-4">
							<div className="space-y-2">
								<label htmlFor="old-password" className="text-sm font-medium">
									Current password
								</label>
								<Input
									id="old-password"
									type="password"
									value={oldPassword}
									onChange={(e) => setOldPassword(e.target.value)}
									autoComplete="current-password"
								/>
							</div>
							<div className="space-y-2">
								<label htmlFor="new-password" className="text-sm font-medium">
									New password
								</label>
								<Input
									id="new-password"
									type="password"
									value={newPassword}
									onChange={(e) => setNewPassword(e.target.value)}
									autoComplete="new-password"
									minLength={8}
								/>
								{newPassword.length > 0 && newPassword.length < 8 && (
									<p className="text-xs text-destructive">
										Must be at least 8 characters
									</p>
								)}
							</div>
							<div className="space-y-2">
								<label
									htmlFor="confirm-password"
									className="text-sm font-medium"
								>
									Confirm new password
								</label>
								<Input
									id="confirm-password"
									type="password"
									value={confirmPassword}
									onChange={(e) => setConfirmPassword(e.target.value)}
									autoComplete="new-password"
								/>
								{passwordMismatch && (
									<p className="text-xs text-destructive">
										Passwords do not match
									</p>
								)}
							</div>
							<Button type="submit" disabled={!canSubmit}>
								{changing ? "Changing..." : "Change password"}
							</Button>
						</form>
					</CardContent>
				</Card>
			</div>
		</>
	);
}
