import { useState } from "react";
import { useNavigate } from "react-router";
import { Copy, Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { MultiSelect } from "@/components/ui/multi-select";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { withToast } from "@/app/lib/api-toast";
import { createApiKey } from "./_actions/actions";
import type { ApiKeyScope } from "@/app/lib/types";

const SCOPE_OPTIONS = [
	{
		value: "workflows_read",
		label: "Workflows Read",
		description: "Read workflow definitions",
	},
	{
		value: "runs_read",
		label: "Runs Read",
		description: "Read runs and their steps",
	},
	{
		value: "runs_write",
		label: "Runs Write",
		description: "Create new runs",
	},
	{
		value: "runs_manage",
		label: "Runs Manage",
		description: "Cancel, approve, reject, retry",
	},
	{
		value: "stats_read",
		label: "Stats Read",
		description: "Read aggregated statistics",
	},
];

export function Component() {
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
						options={SCOPE_OPTIONS}
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
