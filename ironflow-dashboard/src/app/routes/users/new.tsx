import { useState } from "react";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent } from "@/components/ui/card";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { withToast } from "@/app/lib/api-toast";
import { createUser } from "./_actions/actions";

const USERNAME_MIN_LENGTH = 3;
const PASSWORD_MIN_LENGTH = 8;

export function Component() {
	const navigate = useNavigate();
	const [email, setEmail] = useState("");
	const [username, setUsername] = useState("");
	const [password, setPassword] = useState("");
	const [isAdmin, setIsAdmin] = useState(false);
	const [pending, setPending] = useState(false);

	useDocumentMeta({
		title: "New User",
		description: "Create a new user account.",
	});

	function handleCreate() {
		setPending(true);
		withToast(
			createUser({
				email: email.trim(),
				username: username.trim(),
				password,
				is_admin: isAdmin,
			}),
			{
				loading: "Creating user...",
				success: "User created",
				error: "Failed to create user",
			},
		)
			.then(() => navigate("/users"))
			.catch(() => {})
			.finally(() => setPending(false));
	}

	const canCreate =
		email.trim().length > 0 &&
		username.trim().length >= USERNAME_MIN_LENGTH &&
		password.length >= PASSWORD_MIN_LENGTH &&
		!pending;

	return (
		<HeaderApp title="New User" description="Create a new user account.">
			<div className="max-w-lg">
				<Card>
					<CardContent className="p-6">
						<form
							onSubmit={(e) => {
								e.preventDefault();
								if (canCreate) handleCreate();
							}}
							className="space-y-4"
						>
							<div>
								<label htmlFor="user-email" className="text-sm font-medium">
									Email
								</label>
								<Input
									id="user-email"
									type="email"
									placeholder="user@example.com"
									value={email}
									onChange={(e) => setEmail(e.target.value)}
									className="mt-1"
									autoComplete="email"
									aria-required="true"
								/>
							</div>

							<div>
								<label htmlFor="user-username" className="text-sm font-medium">
									Username
								</label>
								<Input
									id="user-username"
									placeholder="johndoe"
									value={username}
									onChange={(e) => setUsername(e.target.value)}
									className="mt-1"
									autoComplete="username"
									aria-required="true"
									aria-describedby="username-hint"
								/>
								<p
									id="username-hint"
									className="text-xs text-muted-foreground mt-1"
								>
									At least {USERNAME_MIN_LENGTH} characters
								</p>
							</div>

							<div>
								<label htmlFor="user-password" className="text-sm font-medium">
									Password
								</label>
								<Input
									id="user-password"
									type="password"
									placeholder=""
									value={password}
									onChange={(e) => setPassword(e.target.value)}
									className="mt-1"
									autoComplete="new-password"
									aria-required="true"
									aria-describedby="password-hint"
								/>
								<p
									id="password-hint"
									className="text-xs text-muted-foreground mt-1"
								>
									At least {PASSWORD_MIN_LENGTH} characters
								</p>
							</div>

							<div className="flex items-start gap-2">
								<Checkbox
									id="user-is-admin"
									checked={isAdmin}
									onCheckedChange={(checked) => setIsAdmin(checked === true)}
									aria-describedby="admin-desc"
									className="mt-0.5"
								/>
								<div>
									<label
										htmlFor="user-is-admin"
										className="text-sm font-medium cursor-pointer"
									>
										Admin privileges
									</label>
									<p
										id="admin-desc"
										className="text-xs text-muted-foreground mt-0.5"
									>
										Grants full access to all workflows, runs, and user
										management.
									</p>
								</div>
							</div>

							<div className="flex gap-3 pt-2">
								<Button
									type="button"
									variant="outline"
									onClick={() => {
										if (window.history.length > 1) {
											navigate(-1);
										} else {
											navigate("/users");
										}
									}}
								>
									Cancel
								</Button>
								<Button type="submit" disabled={!canCreate}>
									{pending ? "Creating..." : "Create User"}
								</Button>
							</div>
						</form>
					</CardContent>
				</Card>
			</div>
		</HeaderApp>
	);
}
