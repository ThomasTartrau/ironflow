import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import type { AccountKindResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { createAccount, listAccountKinds } from "./_actions/actions";

export function Component() {
	const navigate = useNavigate();
	useDocumentMeta({ title: "Add account" });
	const [kinds, setKinds] = useState<AccountKindResponse[]>([]);
	const [kind, setKind] = useState<AccountKindResponse | null>(null);
	const [name, setName] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [token, setToken] = useState("");
	const [tags, setTags] = useState("");
	const [priority, setPriority] = useState("100");
	const [maxConcurrency, setMaxConcurrency] = useState("");
	const [threshold, setThreshold] = useState("0.8");
	const [error, setError] = useState<string | null>(null);
	const [submitting, setSubmitting] = useState(false);

	useEffect(() => {
		listAccountKinds()
			.then(setKinds)
			.catch((e: unknown) =>
				setError(e instanceof Error ? e.message : "failed to load kinds"),
			);
	}, []);

	async function submit(event: React.FormEvent) {
		event.preventDefault();
		if (!kind) return;
		setSubmitting(true);
		setError(null);
		try {
			await createAccount({
				name,
				kind: kind.id,
				token,
				display_name: displayName || null,
				tags: tags
					.split(",")
					.map((t) => t.trim())
					.filter(Boolean),
				priority: Number(priority),
				max_concurrency: maxConcurrency === "" ? null : Number(maxConcurrency),
				alert_threshold: Number(threshold),
			});
			navigate("/accounts");
		} catch (e: unknown) {
			// A 422 carries the provider's verdict on the token: show it in the form.
			setError(e instanceof Error ? e.message : "failed to add account");
		} finally {
			setSubmitting(false);
		}
	}

	const tokenField = kind?.fields.find((f) => f.secret);

	return (
		<div className="space-y-4">
			<HeaderApp
				title="Add account"
				description="Register an AI provider account. The token is checked before it is stored."
			/>
			{!kind ? (
				<div className="grid gap-3 md:grid-cols-2">
					{kinds.map((k) => (
						<Card key={k.id}>
							<CardContent className="space-y-2 pt-4">
								<div className="font-semibold">{k.display_name}</div>
								<div className="text-xs text-muted-foreground">{k.id}</div>
								<Button size="sm" onClick={() => setKind(k)}>
									Choose
								</Button>
							</CardContent>
						</Card>
					))}
				</div>
			) : (
				<Card>
					<CardContent className="pt-4">
						<form className="space-y-3" onSubmit={submit}>
							<Input
								aria-label="Name"
								placeholder="name (e.g. perso-max)"
								required
								pattern="[a-z0-9][a-z0-9-]{0,62}"
								value={name}
								onChange={(e) => setName(e.target.value)}
							/>
							<Input
								aria-label="Display name"
								placeholder="display name"
								value={displayName}
								onChange={(e) => setDisplayName(e.target.value)}
							/>
							<Input
								aria-label={tokenField?.label ?? "Token"}
								type="password"
								autoComplete="off"
								required
								placeholder="sk-ant-oat01-..."
								value={token}
								onChange={(e) => setToken(e.target.value)}
							/>
							<p className="text-xs text-muted-foreground">
								{tokenField?.help ??
									"Run `claude setup-token` and paste the sk-ant-oat01-... token"}
							</p>
							<Input
								aria-label="Tags"
								placeholder="tags, comma separated"
								value={tags}
								onChange={(e) => setTags(e.target.value)}
							/>
							<Input
								aria-label="Priority"
								type="number"
								value={priority}
								onChange={(e) => setPriority(e.target.value)}
							/>
							<Input
								aria-label="Max concurrency"
								type="number"
								min={1}
								placeholder="unlimited"
								value={maxConcurrency}
								onChange={(e) => setMaxConcurrency(e.target.value)}
							/>
							<Input
								aria-label="Alert threshold"
								type="number"
								step="0.05"
								min={0.05}
								max={1}
								value={threshold}
								onChange={(e) => setThreshold(e.target.value)}
							/>
							{error && (
								<p role="alert" className="text-sm text-destructive">
									{error}
								</p>
							)}
							<div className="flex gap-2">
								<Button
									type="button"
									variant="outline"
									onClick={() => setKind(null)}
								>
									Back
								</Button>
								<Button type="submit" disabled={submitting}>
									Add account
								</Button>
							</div>
						</form>
					</CardContent>
				</Card>
			)}
		</div>
	);
}
