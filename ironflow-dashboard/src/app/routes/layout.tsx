import { useEffect } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Outlet, useNavigation, useNavigate } from "react-router";
import { SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import { Loader2 } from "lucide-react";
import { AppSidebar } from "@/app/components/sidebar/app-sidebar";
import { useAppDispatch, useAppSelector } from "@/app/store";
import { fetchCurrentUser } from "@/app/store/auth-slice";

function LayoutSkeleton() {
	return (
		<div className="flex items-center justify-center h-full min-h-[200px]">
			<Loader2 className="size-6 animate-spin text-muted-foreground" />
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
			<div className="flex flex-col flex-1 min-w-0">
				<AnimatePresence>
					{isNavigating && (
						<motion.div
							key="nav-progress"
							className="fixed top-0 left-0 right-0 z-50 h-[2px] bg-primary origin-left"
							initial={{ scaleX: 0 }}
							animate={{ scaleX: 0.85 }}
							exit={{ scaleX: 1, opacity: 0 }}
							transition={{ duration: 0.4, ease: "easeOut" }}
						/>
					)}
				</AnimatePresence>
				<header
					className="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background/95 backdrop-blur-sm sticky top-0 z-10 px-4"
					aria-busy={isNavigating}
					aria-live="polite"
				>
					<SidebarTrigger className="-ml-1" />
				</header>
				<main className="flex-1 overflow-auto min-w-0">
					{auth.status === "authenticated" ? <Outlet /> : <LayoutSkeleton />}
				</main>
			</div>
		</SidebarProvider>
	);
}
