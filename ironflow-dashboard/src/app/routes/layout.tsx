import { useEffect } from "react";
import { Outlet, useNavigation, useNavigate } from "react-router";
import {
	SidebarInset,
	SidebarProvider,
	SidebarTrigger,
} from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { AppSidebar } from "@/app/components/sidebar/app-sidebar";
import { useAppDispatch, useAppSelector } from "@/app/store";
import { fetchCurrentUser } from "@/app/store/auth-slice";

function LayoutSkeleton() {
	return (
		<div className="space-y-6 p-6">
			<div>
				<Skeleton className="h-8 w-48 mb-2" />
				<Skeleton className="h-4 w-64" />
			</div>
			<div className="grid grid-cols-1 md:grid-cols-3 lg:grid-cols-5 gap-4">
				{Array.from({ length: 5 }, (_, i) => (
					<Skeleton key={i} className="h-24 rounded-lg" />
				))}
			</div>
			<Skeleton className="h-6 w-32 mt-2" />
			<Skeleton className="h-64 rounded-lg" />
		</div>
	);
}

export default function Layout() {
	const navigate = useNavigate();
	const navigation = useNavigation();
	const dispatch = useAppDispatch();
	const auth = useAppSelector((state) => state.auth);

	useEffect(() => {
		if (auth.status === "idle") {
			dispatch(fetchCurrentUser());
		}
	}, [auth.status, dispatch]);

	useEffect(() => {
		if (auth.status === "unauthenticated") {
			navigate("/sign-in");
		}
	}, [auth.status, navigate]);

	const isNavigating = navigation.state === "loading";

	return (
		<SidebarProvider>
			<AppSidebar />
			<SidebarInset className="min-w-0">
				<header className="flex h-12 items-center gap-2 border-b px-4">
					<SidebarTrigger className="-ml-1" />
					{isNavigating && (
						<div className="ml-auto h-1 w-24 overflow-hidden rounded-full bg-muted">
							<div className="h-full w-1/2 animate-[shimmer_1s_ease-in-out_infinite] rounded-full bg-primary/40" />
						</div>
					)}
				</header>
				<main className="flex-1 overflow-auto min-w-0">
					{auth.status === "authenticated" ? <Outlet /> : <LayoutSkeleton />}
				</main>
			</SidebarInset>
		</SidebarProvider>
	);
}
