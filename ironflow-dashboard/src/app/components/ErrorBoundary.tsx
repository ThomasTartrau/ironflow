import { useRouteError, isRouteErrorResponse, Link } from "react-router";
import { Button } from "@/components/ui/button";

export function ErrorBoundary() {
	const error = useRouteError();

	let status = 500;
	let title = "Something went wrong";
	let message = "An unexpected error occurred. Please try again.";

	if (isRouteErrorResponse(error)) {
		status = error.status;
		if (error.status === 404) {
			title = "Page not found";
			message = "The page you're looking for doesn't exist or has been moved.";
		} else {
			title = `Error ${error.status}`;
			message = error.statusText || message;
		}
	} else if (error instanceof Error) {
		message = error.message;
	}

	return (
		<div className="flex min-h-screen flex-col items-center justify-center bg-background px-6">
			<div className="mx-auto max-w-md text-center space-y-6">
				<p className="text-7xl font-bold text-muted-foreground/30">{status}</p>
				<div className="space-y-2">
					<h1 className="text-2xl font-bold tracking-tight">{title}</h1>
					<p className="text-sm text-muted-foreground leading-relaxed">
						{message}
					</p>
				</div>
				<div className="flex items-center justify-center gap-3 pt-2">
					<Button variant="outline" onClick={() => window.location.reload()}>
						Try again
					</Button>
					<Link to="/">
						<Button>Back to dashboard</Button>
					</Link>
				</div>
			</div>
		</div>
	);
}
