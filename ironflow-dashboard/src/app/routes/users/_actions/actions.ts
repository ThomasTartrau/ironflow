import { api } from "@/app/lib/api";
import type { CreateUserRequest, UserResponse } from "@/app/lib/types";

export function createUser(body: CreateUserRequest): Promise<UserResponse> {
	return api.post<UserResponse>("/users", body).then((res) => res.data);
}

export function deleteUser(id: string): Promise<void> {
	return api.del<void>(`/users/${id}`).then(() => undefined);
}

export function updateUserRole(
	id: string,
	isAdmin: boolean,
): Promise<UserResponse> {
	return api
		.patch<UserResponse>(`/users/${id}/role`, { is_admin: isAdmin })
		.then((res) => res.data);
}
