import { toast } from "sonner";

export interface ToastMessages {
	loading: string;
	success: string;
	error?: string;
}

/**
 * Wraps a Promise with automatic loading/success/error toasts via sonner.
 * Attaches display-only handlers - the original promise is returned unchanged
 * so callers can still chain .then()/.catch()/.finally() normally.
 */
export function withToast<T>(
	promise: Promise<T>,
	messages: ToastMessages,
): Promise<T> {
	toast.promise(promise, {
		loading: messages.loading,
		success: messages.success,
		error: (err: unknown) => {
			console.error(err);
			return err instanceof Error
				? err.message
				: (messages.error ?? "Something went wrong");
		},
	});
	return promise;
}
