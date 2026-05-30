import { useState } from "react";
import { useLoaderData, useNavigate } from "react-router";
import { Copy, Check, TriangleAlert } from "lucide-react";
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
				<div className="max-w-lg space-y-4">
					<div className="flex items-start gap-2 rounded-[var(--radius)] border border-destructive/40 bg-destructive/10 px-3 py-2.5 text-destructive">
						<TriangleAlert
							className="mt-0.5 size-4 shrink-0"
							aria-hidden="true"
						/>
						<p className="text-sm">
							Copy this key now. You will not be able to see it again after
							leaving this page.
						</p>
					</div>
					<div className="flex items-center gap-2">
						<code className="flex-1 select-all rounded-[var(--radius-sm)] bg-muted px-3 py-2 font-mono text-sm break-anywhere">
							{createdKey}
						</code>
						<Button
							variant="outline"
							size="icon-sm"
							onClick={handleCopy}
							aria-label={copied ? "Copied" : "Copy API key"}
						>
							{copied ? <Check /> : <Copy />}
						</Button>
					</div>
					<span className="sr-only" aria-live="polite" aria-atomic="true">
						{copied ? "API key copied to clipboard" : ""}
					</span>
					<Button onClick={() => navigate("/api-keys")}>
						Done, I copied the key
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
			<div className="max-w-lg">
				<form
					onSubmit={(e) => {
						e.preventDefault();
						if (canCreate) handleCreate();
					}}
					className="space-y-6"
				>
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
						<Button
							type="button"
							variant="outline"
							onClick={() => navigate("/api-keys")}
						>
							Cancel
						</Button>
						<Button type="submit" disabled={!canCreate}>
							{pending ? "Creating..." : "Create API Key"}
						</Button>
					</div>
				</form>
			</div>
		</HeaderApp>
	);
}
