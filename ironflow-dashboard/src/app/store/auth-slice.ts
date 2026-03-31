import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { api } from "@/app/lib/api";

interface User {
	user_id: string;
	email: string;
	username: string;
	is_admin: boolean;
}

type AuthState =
	| { status: "idle" }
	| { status: "loading" }
	| { status: "authenticated"; user: User }
	| { status: "unauthenticated" };

const initialState: AuthState = { status: "idle" };

export const fetchCurrentUser = createAsyncThunk(
	"auth/fetchCurrentUser",
	async () => {
		const response = await api.get<User>("/auth/me");
		return response.data;
	},
);

const authSlice = createSlice({
	name: "auth",
	initialState: initialState as AuthState,
	reducers: {
		logout: () => ({ status: "unauthenticated" as const }),
	},
	extraReducers: (builder) => {
		builder
			.addCase(fetchCurrentUser.pending, () => ({
				status: "loading" as const,
			}))
			.addCase(fetchCurrentUser.fulfilled, (_state, action) => ({
				status: "authenticated" as const,
				user: action.payload,
			}))
			.addCase(fetchCurrentUser.rejected, () => ({
				status: "unauthenticated" as const,
			}));
	},
});

export const { logout } = authSlice.actions;
export default authSlice.reducer;
