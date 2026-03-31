import { describe, it, expect, vi, beforeEach } from "vitest";
import { configureStore } from "@reduxjs/toolkit";
import authReducer, { fetchCurrentUser, logout } from "./auth-slice";

function createTestStore() {
	return configureStore({
		reducer: { auth: authReducer },
	});
}

describe("auth-slice", () => {
	let store: ReturnType<typeof createTestStore>;

	beforeEach(() => {
		store = createTestStore();
		vi.restoreAllMocks();
	});

	it("starts with idle status", () => {
		expect(store.getState().auth).toEqual({ status: "idle" });
	});

	it("transitions to loading on fetchCurrentUser.pending", () => {
		store.dispatch({ type: fetchCurrentUser.pending.type });
		expect(store.getState().auth.status).toBe("loading");
	});

	it("transitions to authenticated on fetchCurrentUser.fulfilled", () => {
		const user = {
			user_id: "abc-123",
			email: "alice@example.com",
			username: "alice",
			is_admin: false,
		};
		store.dispatch({ type: fetchCurrentUser.fulfilled.type, payload: user });
		const state = store.getState().auth;
		expect(state.status).toBe("authenticated");
		if (state.status === "authenticated") {
			expect(state.user).toEqual(user);
		}
	});

	it("transitions to unauthenticated on fetchCurrentUser.rejected", () => {
		store.dispatch({ type: fetchCurrentUser.rejected.type });
		expect(store.getState().auth.status).toBe("unauthenticated");
	});

	it("transitions to unauthenticated on logout", () => {
		const user = {
			user_id: "abc-123",
			email: "alice@example.com",
			username: "alice",
			is_admin: false,
		};
		store.dispatch({ type: fetchCurrentUser.fulfilled.type, payload: user });
		expect(store.getState().auth.status).toBe("authenticated");

		store.dispatch(logout());
		expect(store.getState().auth.status).toBe("unauthenticated");
	});
});
