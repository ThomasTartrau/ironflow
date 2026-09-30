import { api } from "@/app/lib/api";
import type {
	AccountKindResponse,
	CreateProviderAccountRequest,
	ProviderAccountResponse,
	ProviderAccountTestResponse,
	ProviderAccountUsageResponse,
	UpdateProviderAccountRequest,
} from "@/app/lib/types";

const base = "/provider-accounts";

function accountPath(account: string): string {
	return `${base}/${encodeURIComponent(account)}`;
}

export function listAccounts(): Promise<ProviderAccountResponse[]> {
	return api
		.get<ProviderAccountResponse[]>(`${base}?per_page=100`)
		.then((res) => res.data);
}

export function getAccount(account: string): Promise<ProviderAccountResponse> {
	return api
		.get<ProviderAccountResponse>(accountPath(account))
		.then((res) => res.data);
}

export function createAccount(
	body: CreateProviderAccountRequest,
): Promise<ProviderAccountResponse> {
	return api.post<ProviderAccountResponse>(base, body).then((res) => res.data);
}

export function updateAccount(
	account: string,
	body: UpdateProviderAccountRequest,
): Promise<ProviderAccountResponse> {
	return api
		.patch<ProviderAccountResponse>(accountPath(account), body)
		.then((res) => res.data);
}

export function deleteAccount(account: string): Promise<void> {
	return api.del<void>(accountPath(account)).then(() => undefined);
}

export function testAccount(
	account: string,
): Promise<ProviderAccountTestResponse> {
	return api
		.post<ProviderAccountTestResponse>(`${accountPath(account)}/test`)
		.then((res) => res.data);
}

export function getAccountUsage(
	account: string,
	days = 30,
): Promise<ProviderAccountUsageResponse> {
	return api
		.get<ProviderAccountUsageResponse>(
			`${accountPath(account)}/usage?days=${days}`,
		)
		.then((res) => res.data);
}

export function listAccountKinds(): Promise<AccountKindResponse[]> {
	return api
		.get<AccountKindResponse[]>(`${base}/kinds`)
		.then((res) => res.data);
}
