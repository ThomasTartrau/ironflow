import { useState } from "react";
import { useLoaderData, useNavigate } from "react-router";
import { Copy, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MultiSelect } from "@/components/ui/multi-select";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { withToast } from "@/app/lib/api-toast";
import { createApiKey } from "./_actions/actions";
import { api } from "@/app/lib/api";
import type { ApiKeyScope, ScopeEntry } from "@/app/lib/types";

export async function loader() {
	const res = await api.get<ScopeEntry[]>("/api-keys/scopes");
	return { availableScopes: res.data };
}

export function Component() {
	const { availableScopes } = useLoaderData<typeof loader>();
	const navigate = useNavigate();
	const [name, setName] = useState("");
	const [scopes, setScopes] = useState<string[]>([]);
	const [pending, setPending] = useState(false);
	const [createdKey, setCreatedKey] = useState<string | null>(null);
	const [copied, setCopied] = useState(false);

	useDocumentMeta({
		title: "New API Key",
		description: "Create a new API key for programmatic access.",
	});

	const scopeOptions = availableScopes.map((s) => ({
		value: s.value,
		label: s.label,
		description: s.description,
	}));

	function handleCreate() {
		setPending(true);
		withToast(
			createApiKey({
				name: name.trim(),
				scopes: scopes as ApiKeyScope[],
			}),
			{
				loading: "Creating API key...",
				success: "API key created",
			},
		)
			.then((result) => {
				setCreatedKey(result.key);
			})
			.catch(() => {})
			.finally(() => setPending(false));
	}

	function handleCopy() {
		if (!createdKey) return;
		navigator.clipboard.writeText(createdKey).then(() => {
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		});
	}

	const canCreate = name.trim().length > 0 && scopes.length > 0 && !pending;

	if (createdKey) {
		return (
			<HeaderApp
				title="API Key Created"
				description="Copy this key now. You will not be able to see it again."
			>
				<div className="max-w-lg space-y-6">
					<div className="flex items-center gap-2">
						<code className="flex-1 rounded bg-muted px-3 py-2 text-sm font-mono break-all">
							{createdKey}
						</code>
						<Button variant="outline" size="icon-sm" onClick={handleCopy}>
							{copied ? (
								<Check className="h-4 w-4 text-green-500" />
							) : (
								<Copy className="h-4 w-4" />
							)}
						</Button>
					</div>
					<Button onClick={() => navigate("/api-keys")}>
						Back to API Keys
					</Button>
				</div>
			</HeaderApp>
		);
	}

	return (
		<HeaderApp
			title="New API Key"
			description="Create a new API key for programmatic access."
		>
			<div className="max-w-lg space-y-6">
				<div>
					<label htmlFor="api-key-name" className="text-sm font-medium">
						Name
					</label>
					<Input
						id="api-key-name"
						placeholder="My integration key"
						value={name}
						onChange={(e) => setName(e.target.value)}
						className="mt-1"
					/>
				</div>

				<div>
					<label htmlFor="scopes" className="text-sm font-medium">
						Scopes
					</label>
					<MultiSelect
						id="scopes"
						options={scopeOptions}
						value={scopes}
						onChange={setScopes}
						placeholder="Select scopes..."
						className="mt-1"
					/>
				</div>

				<div className="flex gap-3">
					<Button variant="outline" onClick={() => navigate("/api-keys")}>
						Cancel
					</Button>
					<Button onClick={handleCreate} disabled={!canCreate}>
						{pending ? "Creating..." : "Create API Key"}
					</Button>
				</div>
			</div>
		</HeaderApp>
	);
}
