import { useState } from "react";
import { useNavigate } from "react-router";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { withToast } from "@/app/lib/api-toast";
import { createSecret } from "./_actions/actions";

export function Component() {
	const navigate = useNavigate();
	const [key, setKey] = useState("");
	const [value, setValue] = useState("");
	const [pending, setPending] = useState(false);

	useDocumentMeta({
		title: "New Secret",
		description: "Create a new encrypted secret.",
	});

	function handleCreate() {
		setPending(true);
		withToast(
			createSecret({
				key: key.trim(),
				value: value.trim(),
			}),
			{
				loading: "Creating secret...",
				success: "Secret created",
				error: "Failed to create secret",
			},
		)
			.then(() => navigate("/secrets"))
			.catch(() => {})
			.finally(() => setPending(false));
	}

	const canCreate =
		key.trim().length > 0 && value.trim().length > 0 && !pending;

	return (
		<HeaderApp title="New Secret" description="Create a new encrypted secret.">
			<div className="max-w-lg space-y-6">
				<div>
					<label htmlFor="secret-key" className="text-sm font-medium">
						Key
					</label>
					<Input
						id="secret-key"
						placeholder="e.g., STRIPE_SECRET_KEY"
						value={key}
						onChange={(e) => setKey(e.target.value)}
						className="mt-1"
					/>
					<p className="text-xs text-muted-foreground mt-1">
						A unique identifier for this secret
					</p>
				</div>

				<div>
					<label htmlFor="secret-value" className="text-sm font-medium">
						Value
					</label>
					<Textarea
						id="secret-value"
						placeholder="Enter the secret value (API key, token, etc.)"
						value={value}
						onChange={(e) => setValue(e.target.value)}
						className="mt-1"
						rows={4}
					/>
					<p className="text-xs text-muted-foreground mt-1">
						Store tokens, API keys, passwords, or any sensitive data
					</p>
				</div>

				<div className="flex gap-3">
					<Button variant="outline" onClick={() => navigate("/secrets")}>
						Cancel
					</Button>
					<Button onClick={handleCreate} disabled={!canCreate}>
						{pending ? "Creating..." : "Create Secret"}
					</Button>
				</div>
			</div>
		</HeaderApp>
	);
}
