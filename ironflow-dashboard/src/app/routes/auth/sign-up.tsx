import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import {
	InputGroup,
	InputGroupInput,
	InputGroupButton,
} from "@/components/ui/input-group";
import { api } from "@/app/lib/api";
import { withToast } from "@/app/lib/api-toast";
import { useAppDispatch } from "@/app/store";
import { fetchCurrentUser } from "@/app/store/auth-slice";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useBranding } from "@/app/lib/branding";
import { Loader2, Eye, EyeOff } from "lucide-react";

type FormState =
	| { status: "idle" }
	| { status: "submitting" }
	| { status: "error"; message: string };

export function Component() {
	const [email, setEmail] = useState("");
	const [username, setUsername] = useState("");
	const [password, setPassword] = useState("");
	const [showPassword, setShowPassword] = useState(false);
	const [formState, setFormState] = useState<FormState>({ status: "idle" });
	const navigate = useNavigate();
	const dispatch = useAppDispatch();
	const branding = useBranding();
	useDocumentMeta({
		title: "Sign up",
		description: `Create your ${branding.name} account.`,
	});

	const handleSubmit = (e: React.FormEvent) => {
		e.preventDefault();
		setFormState({ status: "submitting" });

		withToast(
			api
				.post("/auth/sign-up", { email, username, password })
				.then(() => dispatch(fetchCurrentUser())),
			{
				loading: "Creating account...",
				success: "Account created!",
				error: "Sign up failed",
			},
		)
			.then(() => navigate("/"))
			.catch((err: unknown) => {
				const message =
					err instanceof Error
						? err.message
						: "Sign up failed. Please try again.";
				setFormState({ status: "error", message });
			})
			.finally(() => {
				setFormState((prev) =>
					prev.status === "submitting" ? { status: "idle" } : prev,
				);
			});
	};

	const isSubmitting = formState.status === "submitting";

	return (
		<div className="grid min-h-screen lg:grid-cols-2">
			{/* Left panel */}
			<div className="relative hidden lg:flex flex-col justify-between bg-sidebar border-r border-sidebar-border p-10 overflow-hidden">
				<div className="relative flex items-center gap-3">
					<img
						src={branding.logoUrl}
						alt={branding.name}
						className="size-9 rounded-[var(--radius-sm)]"
						width={36}
						height={36}
						fetchPriority="high"
					/>
					<span className="text-xl font-semibold tracking-tight text-sidebar-foreground">
						{branding.name}
					</span>
				</div>

				<div className="relative space-y-3">
					<p className="text-sm text-sidebar-foreground/60 leading-relaxed max-w-xs">
						Orchestrate workflows, automate operations, and monitor executions
						from a single control plane.
					</p>
					<span className="inline-block font-mono text-xs text-muted-foreground border border-border rounded-[var(--radius-sm)] px-2 py-0.5">
						{branding.name}
					</span>
				</div>

				<p className="relative text-xs text-muted-foreground">
					&copy; {new Date().getFullYear()} {branding.copyright}
				</p>
			</div>

			{/* Right panel -- form */}
			<div className="relative flex flex-col p-6 sm:p-10 overflow-hidden">
				<div className="relative flex items-center gap-3 lg:hidden mb-auto">
					<img
						src={branding.logoUrl}
						alt={branding.name}
						className="size-9 rounded-[var(--radius-sm)]"
						width={36}
						height={36}
						fetchPriority="high"
					/>
					<span className="text-xl font-semibold tracking-tight">
						{branding.name}
					</span>
				</div>

				<div className="relative flex flex-1 flex-col items-center justify-center">
					<div className="w-full max-w-sm space-y-8">
						<div className="space-y-2">
							<span className="block text-xs font-mono uppercase tracking-widest text-primary mb-2">
								{branding.name}
							</span>
							<h1 className="text-2xl font-semibold tracking-tight">
								Create account
							</h1>
							<p className="text-muted-foreground text-sm">
								Sign up to start orchestrating workflows
							</p>
						</div>

						<form
							onSubmit={handleSubmit}
							className="space-y-4"
							aria-busy={isSubmitting}
						>
							<div className="space-y-1.5">
								<label htmlFor="email" className="text-sm font-medium">
									Email
								</label>
								<InputGroup>
									<InputGroupInput
										id="email"
										type="email"
										placeholder="you@example.com"
										value={email}
										onChange={(e) => setEmail(e.target.value)}
										required
										autoComplete="email"
									/>
								</InputGroup>
							</div>

							<div className="space-y-1.5">
								<label htmlFor="username" className="text-sm font-medium">
									Username
								</label>
								<InputGroup>
									<InputGroupInput
										id="username"
										type="text"
										placeholder="johndoe"
										value={username}
										onChange={(e) => setUsername(e.target.value)}
										required
										minLength={3}
										autoComplete="username"
										aria-describedby="username-hint"
									/>
								</InputGroup>
								<p id="username-hint" className="text-xs text-muted-foreground">
									At least 3 characters
								</p>
							</div>

							<div className="space-y-1.5">
								<label htmlFor="password" className="text-sm font-medium">
									Password
								</label>
								<InputGroup>
									<InputGroupInput
										id="password"
										type={showPassword ? "text" : "password"}
										placeholder=""
										value={password}
										onChange={(e) => setPassword(e.target.value)}
										required
										minLength={8}
										autoComplete="new-password"
										aria-describedby="password-hint"
									/>
									<InputGroupButton
										size="icon-sm"
										type="button"
										aria-label={
											showPassword ? "Hide password" : "Show password"
										}
										onClick={() => setShowPassword((v) => !v)}
									>
										{showPassword ? (
											<EyeOff className="size-4" aria-hidden="true" />
										) : (
											<Eye className="size-4" aria-hidden="true" />
										)}
									</InputGroupButton>
								</InputGroup>
								<p id="password-hint" className="text-xs text-muted-foreground">
									At least 8 characters
								</p>
							</div>

							<Button
								type="submit"
								className="w-full"
								size="lg"
								disabled={isSubmitting}
							>
								{isSubmitting && (
									<Loader2 className="size-4 animate-spin" aria-hidden="true" />
								)}
								{isSubmitting ? "Creating account..." : "Create account"}
							</Button>

							{formState.status === "error" && (
								<p className="text-sm text-destructive mt-2" role="alert">
									{formState.message}
								</p>
							)}
						</form>

						<p className="text-center text-sm text-muted-foreground">
							Already have an account?{" "}
							<Link
								to="/sign-in"
								className="font-medium text-foreground underline underline-offset-2 hover:text-primary"
							>
								Sign in
							</Link>
						</p>
					</div>
				</div>
			</div>
		</div>
	);
}
