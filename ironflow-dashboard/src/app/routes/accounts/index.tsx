import { useLoaderData, useNavigate, useRevalidator } from "react-router";
import { Gauge, Plus } from "lucide-react";
import type { ProviderAccountResponse } from "@/app/lib/types";
import { HeaderApp } from "@/app/components/HeaderApp";
import { useDocumentMeta } from "@/app/hooks/use-document-meta";
import { useRevalidateOnEvent } from "@/app/hooks/use-revalidate-on-event";
import { withToast } from "@/app/lib/api-toast";
import { Button } from "@/components/ui/button";
import {
	deleteAccount,
	listAccounts,
	testAccount,
	updateAccount,
} from "./_actions/actions";
import { AccountCard, type AccountEdit } from "./_components/AccountCard";

export async function loader() {
	return { accounts: await listAccounts() };
}

export function Component() {
	const { accounts } = useLoaderData() as {
		accounts: ProviderAccountResponse[];
	};
	const navigate = useNavigate();
	const revalidator = useRevalidator();
	useDocumentMeta({ title: "Accounts" });
	useRevalidateOnEvent({
		types: ["provider_account.updated", "provider_account.usage_updated"],
	});

	function run<T>(promise: Promise<T>, loading: string, success: string) {
		withToast(promise, { loading, success })
			.catch(() => undefined)
			.finally(() => revalidator.revalidate());
	}

	return (
		<HeaderApp
			title="Accounts"
			description="AI provider accounts, their tokens and their usage limits."
			titleItem={
				<Button size="sm" onClick={() => navigate("/accounts/new")}>
					<Plus className="h-4 w-4" /> Add account
				</Button>
			}
		>
			{accounts.length === 0 ? (
				<div className="flex flex-col items-center gap-2 py-16 text-sm text-muted-foreground">
					<Gauge className="h-8 w-8" />
					No provider account yet: agent steps use the worker environment.
				</div>
			) : (
				<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
					{accounts.map((account) => (
						<AccountCard
							key={account.id}
							account={account}
							onUpdate={(edit: AccountEdit) =>
								run(
									updateAccount(account.id, edit),
									"Saving...",
									"Account updated",
								)
							}
							onTest={() =>
								run(
									testAccount(account.id).then((test) => {
										// A rejected token is a successful call: surface it as an error toast.
										if (test.result === "unauthorized") {
											throw new Error(
												"Token rejected by the provider: generate a new one and edit the account",
											);
										}
										return test;
									}),
									"Testing token...",
									"Token valid",
								)
							}
							onDelete={() =>
								run(deleteAccount(account.id), "Deleting...", "Account deleted")
							}
						/>
					))}
				</div>
			)}
		</HeaderApp>
	);
}
