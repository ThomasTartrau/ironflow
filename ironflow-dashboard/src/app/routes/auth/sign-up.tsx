import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/app/lib/api";
import { withToast } from "@/app/lib/api-toast";
import { useAppDispatch } from "@/app/store";
import { fetchCurrentUser } from "@/app/store/auth-slice";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Workflow, Zap, Shield } from "lucide-react";

const FEATURES = [
	{
		icon: Workflow,
		title: "Workflow Orchestration",
		description:
			"Build and run multi-step workflows with shell, HTTP, and AI agent steps.",
	},
	{
		icon: Zap,
		title: "Real-time Monitoring",
		description:
			"Track workflow executions, costs, and durations from a single dashboard.",
	},
	{
		icon: Shield,
		title: "Built-in AI Agents",
		description:
			"Leverage Claude to analyze, summarize, and act on workflow data.",
	},
];

export function Component() {
	const [email, setEmail] = useState("");
	const [username, setUsername] = useState("");
	const [password, setPassword] = useState("");
	const [loading, setLoading] = useState(false);
	const navigate = useNavigate();
	const dispatch = useAppDispatch();
	useDocumentMeta({
		title: "Sign up",
		description: "Create your Ironflow account.",
	});

	const handleSubmit = (e: React.FormEvent) => {
		e.preventDefault();
		setLoading(true);

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
			.catch(() => {})
			.finally(() => setLoading(false));
	};

	return (
		<div className="grid min-h-screen lg:grid-cols-2">
			<div className="relative hidden lg:flex flex-col justify-between bg-gradient-to-b from-neutral-900 to-neutral-950 p-10 text-white overflow-hidden">
				<div className="absolute -top-24 -left-24 size-96 rounded-full bg-primary/20 blur-3xl" />
				<div className="absolute -bottom-32 -right-32 size-96 rounded-full bg-primary/15 blur-3xl" />

				<div className="relative flex items-center gap-3">
					<img src="/logo.svg" alt="ironflow" className="size-9 rounded-lg" />
					<span className="text-xl font-bold tracking-tight">ironflow</span>
				</div>

				<div className="relative space-y-8">
					{FEATURES.map((feature) => (
						<div key={feature.title} className="flex gap-4">
							<div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-white/10 bg-white/5 backdrop-blur-sm">
								<feature.icon className="size-5 text-primary" />
							</div>
							<div>
								<h3 className="font-semibold">{feature.title}</h3>
								<p className="text-neutral-400 text-sm mt-1">
									{feature.description}
								</p>
							</div>
						</div>
					))}
				</div>

				<p className="relative text-xs text-neutral-500">
					&copy; {new Date().getFullYear()} ironflow
				</p>
			</div>

			<div className="relative flex flex-col p-6 sm:p-10 overflow-hidden">
				<div className="relative flex items-center gap-3 lg:hidden mb-auto">
					<img src="/logo.svg" alt="ironflow" className="size-9 rounded-lg" />
					<span className="text-xl font-bold tracking-tight">ironflow</span>
				</div>

				<div className="relative flex flex-1 flex-col items-center justify-center">
					<div className="w-full max-w-sm space-y-8">
						<div className="space-y-2 text-center lg:text-left">
							<h1 className="text-2xl font-bold tracking-tight">
								Create account
							</h1>
							<p className="text-muted-foreground text-sm">
								Sign up to start orchestrating workflows
							</p>
						</div>

						<form onSubmit={handleSubmit} className="space-y-4">
							<div className="space-y-2">
								<label htmlFor="email" className="text-sm font-medium">
									Email
								</label>
								<Input
									id="email"
									type="email"
									placeholder="you@example.com"
									value={email}
									onChange={(e) => setEmail(e.target.value)}
									required
								/>
							</div>

							<div className="space-y-2">
								<label htmlFor="username" className="text-sm font-medium">
									Username
								</label>
								<Input
									id="username"
									type="text"
									placeholder="johndoe"
									value={username}
									onChange={(e) => setUsername(e.target.value)}
									required
									minLength={3}
								/>
							</div>

							<div className="space-y-2">
								<label htmlFor="password" className="text-sm font-medium">
									Password
								</label>
								<Input
									id="password"
									type="password"
									placeholder="Min 8 characters"
									value={password}
									onChange={(e) => setPassword(e.target.value)}
									required
									minLength={8}
								/>
							</div>

							<Button
								type="submit"
								className="w-full"
								size="lg"
								disabled={loading}
							>
								{loading ? "Creating account..." : "Create account"}
							</Button>
						</form>

						<p className="text-center text-sm text-muted-foreground">
							Already have an account?{" "}
							<Link
								to="/sign-in"
								className="font-medium text-foreground hover:underline"
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
