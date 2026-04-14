import { createBrowserRouter, Navigate } from "react-router";
import Layout from "./routes/layout";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { DashboardSkeleton } from "./components/skeletons/DashboardSkeleton";
import { RunsListSkeleton } from "./components/skeletons/RunsListSkeleton";
import { RunDetailSkeleton } from "./components/skeletons/RunDetailSkeleton";
import { WorkflowsListSkeleton } from "./components/skeletons/WorkflowsSkeleton";
import { WorkflowDetailSkeleton } from "./components/skeletons/WorkflowDetailSkeleton";
import { ApiKeysListSkeleton } from "./components/skeletons/ApiKeysSkeleton";

export const router = createBrowserRouter([
	{
		path: "/sign-in",
		lazy: () => import("./routes/auth/sign-in"),
		errorElement: <ErrorBoundary />,
	},
	{
		path: "/sign-up",
		lazy: () => import("./routes/auth/sign-up"),
		errorElement: <ErrorBoundary />,
	},

	{
		element: <Layout />,
		errorElement: <ErrorBoundary />,
		children: [
			{
				path: "/",
				lazy: () => import("./routes/dashboard"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: DashboardSkeleton,
			},
			{
				path: "/workflows",
				lazy: () => import("./routes/workflows"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: WorkflowsListSkeleton,
			},
			{
				path: "/workflows/:name",
				lazy: () => import("./routes/workflows/$name"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: WorkflowDetailSkeleton,
			},
			{
				path: "/runs",
				lazy: () => import("./routes/runs"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: RunsListSkeleton,
			},

			{
				path: "/runs/:id",
				lazy: () => import("./routes/runs/$id"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: RunDetailSkeleton,
			},
			{
				path: "/api-keys",
				lazy: () => import("./routes/api-keys"),
				errorElement: <ErrorBoundary />,
				HydrateFallback: ApiKeysListSkeleton,
			},
			{
				path: "/api-keys/new",
				lazy: () => import("./routes/api-keys/new"),
				errorElement: <ErrorBoundary />,
			},
			{
				path: "/users",
				lazy: () => import("./routes/users"),
				errorElement: <ErrorBoundary />,
			},
			{
				path: "/users/new",
				lazy: () => import("./routes/users/new"),
				errorElement: <ErrorBoundary />,
			},
			{
				path: "*",
				element: <Navigate to="/" replace />,
			},
		],
	},
]);
