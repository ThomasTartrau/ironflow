import { api } from "@/app/lib/api";
import type {
	CreateApiKeyRequest,
	CreateApiKeyResponse,
} from "@/app/lib/types";

export function createApiKey(
	body: CreateApiKeyRequest,
): Promise<CreateApiKeyResponse> {
	return api
		.post<CreateApiKeyResponse>("/api-keys", body)
		.then((res) => res.data);
}

export function deleteApiKey(id: string): Promise<void> {
	return api.del<void>(`/api-keys/${id}`).then(() => undefined);
}
