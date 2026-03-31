import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router";
import { NuqsAdapter } from "nuqs/adapters/react-router/v7";
import { Provider } from "react-redux";
import { store } from "./app/store";
import { router } from "./app/router";
import { Toaster } from "@/components/ui/sonner";
import "./index.css";

createRoot(document.getElementById("root")!).render(
	<StrictMode>
		<NuqsAdapter>
			<Provider store={store}>
				<RouterProvider router={router} />
				<Toaster position="bottom-right" />
			</Provider>
		</NuqsAdapter>
	</StrictMode>,
);
