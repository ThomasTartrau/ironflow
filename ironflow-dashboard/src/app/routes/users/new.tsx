import { useState } from "react";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { withToast } from "@/app/lib/api-toast";
import { createUser } from "./_actions/actions";

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
		username.trim().length >= 3 &&
		password.length >= 8 &&
		!pending;

	return (
		<HeaderApp title="New User" description="Create a new user account.">
			<div className="max-w-lg space-y-6">
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
					/>
					<p className="text-xs text-muted-foreground mt-1">
						At least 3 characters
					</p>
				</div>

				<div>
					<label htmlFor="user-password" className="text-sm font-medium">
						Password
					</label>
					<Input
						id="user-password"
						type="password"
						placeholder="Min. 8 characters"
						value={password}
						onChange={(e) => setPassword(e.target.value)}
						className="mt-1"
					/>
				</div>

				<div className="flex items-center gap-2">
					<Checkbox
						id="user-is-admin"
						checked={isAdmin}
						onCheckedChange={(checked) => setIsAdmin(checked === true)}
					/>
					<label
						htmlFor="user-is-admin"
						className="text-sm font-medium cursor-pointer"
					>
						Admin privileges
					</label>
				</div>

				<div className="flex gap-3">
					<Button variant="outline" onClick={() => navigate("/users")}>
						Cancel
					</Button>
					<Button onClick={handleCreate} disabled={!canCreate}>
						{pending ? "Creating..." : "Create User"}
					</Button>
				</div>
			</div>
		</HeaderApp>
	);
}
