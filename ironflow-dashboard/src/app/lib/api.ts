export const API_BASE = import.meta.env.VITE_API_URL ?? "";

export class ApiError extends Error {
	status: number;

	constructor(status: number, message: string) {
		super(message);
		this.status = status;
	}
}

export interface ApiResponse<T> {
	data: T;
	meta?: { page: number; per_page: number; total: number };
}

function throwApiError(response: Response): Promise<never> {
	return response
		.json()
		.catch(() => ({}))
		.then((body: Record<string, unknown>) => {
			throw new ApiError(
				response.status,
				(body?.error as Record<string, string>)?.message ?? response.statusText,
			);
		});
}

const AUTH_SKIP_PATHS = [
	"/auth/refresh",
	"/auth/sign-out",
	"/auth/sign-in",
	"/auth/sign-up",
];

let refreshPromise: Promise<boolean> | null = null;

function doRefresh(): Promise<boolean> {
	if (refreshPromise) return refreshPromise;

	refreshPromise = fetch(`${API_BASE}/api/v1/auth/refresh`, {
		method: "POST",
		credentials: "include",
		headers: { Accept: "application/json" },
	})
		.then((res) => res.ok)
		.catch(() => false)
		.then((success) => {
			refreshPromise = null;
			return success;
		});

	return refreshPromise;
}

export function customFetch(
	path: string,
	options: RequestInit = {},
): Promise<Response> {
	const { headers, ...rest } = options;
	return fetch(`${API_BASE}/api/v1${path}`, {
		credentials: "include",
		...rest,
		headers: {
			"Content-Type": "application/json",
			Accept: "application/json",
			...headers,
		},
	});
}

function request<T>(
	path: string,
	options: RequestInit = {},
): Promise<ApiResponse<T>> {
	return customFetch(path, options).then((response) => {
		if (response.status === 401 && !AUTH_SKIP_PATHS.includes(path)) {
			return doRefresh().then((refreshed) => {
				if (!refreshed) {
					return throwApiError(response);
				}
				return customFetch(path, options).then((retryResponse) => {
					if (!retryResponse.ok) {
						return throwApiError(retryResponse);
					}
					if (retryResponse.status === 204) {
						return { data: undefined as T };
					}
					return retryResponse.json() as Promise<ApiResponse<T>>;
				});
			});
		}

		if (!response.ok) {
			return throwApiError(response);
		}

		if (response.status === 204) {
			return { data: undefined as T };
		}

		return response.json() as Promise<ApiResponse<T>>;
	});
}

export const api = {
	get: <T>(path: string) => request<T>(path),
	post: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: "POST",
			body: body != null ? JSON.stringify(body) : undefined,
		}),
	del: <T>(path: string) => request<T>(path, { method: "DELETE" }),
	patch: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: "PATCH",
			body: body != null ? JSON.stringify(body) : undefined,
		}),
	put: <T>(path: string, body?: unknown) =>
		request<T>(path, {
			method: "PUT",
			body: body != null ? JSON.stringify(body) : undefined,
		}),
};
