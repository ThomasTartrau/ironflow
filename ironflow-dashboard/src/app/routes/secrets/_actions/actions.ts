import { api } from "@/app/lib/api";
import type {
	SecretResponse,
	SetSecretRequest,
	UpdateSecretRequest,
	ListSecretsQuery,
} from "@/app/lib/types";

export function listSecrets(query?: ListSecretsQuery) {
	const params = new URLSearchParams();
	if (query?.prefix) params.append("prefix", query.prefix);
	if (query?.page) params.append("page", String(query.page));
	if (query?.per_page) params.append("per_page", String(query.per_page));

	const queryString = params.toString();
	const path = queryString ? `/secrets?${queryString}` : "/secrets";

	return api.get<SecretResponse[]>(path).then((res) => ({
		data: res.data,
		meta: res.meta,
	}));
}

export function createSecret(body: SetSecretRequest): Promise<SecretResponse> {
	return api.post<SecretResponse>("/secrets", body).then((res) => res.data);
}

export function updateSecret(
	key: string,
	body: UpdateSecretRequest,
): Promise<SecretResponse> {
	return api
		.put<SecretResponse>(`/secrets/${encodeURIComponent(key)}`, body)
		.then((res) => res.data);
}

export function deleteSecret(key: string): Promise<void> {
	return api
		.del<void>(`/secrets/${encodeURIComponent(key)}`)
		.then(() => undefined);
}
