import { useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { ChevronRight } from "lucide-react";
import type { AccountKindResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { createAccount, listAccountKinds } from "./_actions/actions";
import { kindDescription } from "./_components/account-state";
import {
	AccountFormFields,
	EMPTY_ACCOUNT_FORM,
	parseAccountForm,
} from "./_components/AccountFormFields";

export function Component() {
	const navigate = useNavigate();
	useDocumentMeta({ title: "Add account" });
	const [kinds, setKinds] = useState<AccountKindResponse[]>([]);
	const [kind, setKind] = useState<AccountKindResponse | null>(null);
	const [values, setValues] = useState(EMPTY_ACCOUNT_FORM);
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
				...parseAccountForm(values),
				name: values.name,
				kind: kind.id,
				token: values.token,
				display_name: values.displayName || null,
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
		<HeaderApp
			title="Add account"
			description={
				kind
					? `New ${kind.display_name} account. The token is checked before it is stored.`
					: "Choose the kind of account to register."
			}
		>
			{!kind ? (
				<div className="grid gap-3 md:grid-cols-2">
					{kinds.map((k) => (
						<button
							key={k.id}
							type="button"
							onClick={() => setKind(k)}
							className="group rounded-[var(--radius-xl)] text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						>
							<Card className="h-full py-4 shadow-none transition-colors group-hover:border-primary/50 group-hover:bg-muted/40">
								<CardContent className="flex items-center gap-4 px-4">
									<div className="flex-1 space-y-1">
										<div className="font-semibold">{k.display_name}</div>
										{kindDescription(k.id) && (
											<p className="text-sm text-muted-foreground">
												{kindDescription(k.id)}
											</p>
										)}
									</div>
									<ChevronRight
										className="size-5 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5"
										aria-hidden="true"
									/>
								</CardContent>
							</Card>
						</button>
					))}
				</div>
			) : (
				<Card className="max-w-lg shadow-none">
					<CardContent className="p-6">
						<form className="space-y-4" onSubmit={submit}>
							<AccountFormFields
								mode="create"
								idPrefix="account"
								values={values}
								onChange={setValues}
								tokenLabel={tokenField?.label}
								tokenHint={tokenField?.help}
							/>
							{error && (
								<p role="alert" className="text-sm text-destructive">
									{error}
								</p>
							)}
							<div className="flex gap-3 pt-2">
								<Button
									type="button"
									variant="outline"
									onClick={() => setKind(null)}
								>
									Back
								</Button>
								<Button type="submit" disabled={submitting}>
									{submitting ? "Checking token..." : "Add account"}
								</Button>
							</div>
						</form>
					</CardContent>
				</Card>
			)}
		</HeaderApp>
	);
}
