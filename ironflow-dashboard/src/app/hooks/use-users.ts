import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { UserResponse } from "../lib/types";

const PER_PAGE = 100;

type UsersState =
	| { status: "idle" }
	| { status: "loading" }
	| { status: "success"; users: UserResponse[] }
	| { status: "error"; error: string };

interface UseUsersOptions {
	/** Fetch only when true. `GET /users` is admin-only, so members must opt out. */
	enabled: boolean;
}

/**
 * Fetch the user list for author pickers.
 *
 * Stays `idle` while disabled, so a non-admin never issues a request that would
 * come back 403.
 */
export function useUsers({ enabled }: UseUsersOptions): UsersState {
	const [state, setState] = useState<UsersState>({ status: "idle" });

	useEffect(() => {
		if (!enabled) {
			setState({ status: "idle" });
			return;
		}

		let cancelled = false;
		setState({ status: "loading" });

		api
			.get<UserResponse[]>(`/users?page=1&per_page=${PER_PAGE}`)
			.then((res) => {
				if (!cancelled) setState({ status: "success", users: res.data });
			})
			.catch((error: unknown) => {
				if (cancelled) return;
				const message =
					error instanceof Error ? error.message : "failed to load users";
				setState({ status: "error", error: message });
			});

		return () => {
			cancelled = true;
		};
	}, [enabled]);

	return state;
}
