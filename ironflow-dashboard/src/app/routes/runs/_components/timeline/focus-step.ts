const pendingTimers = new Set<ReturnType<typeof setTimeout>>();

export function focusStep(
	stepId: string,
	ownerRunId: string,
	currentRunId: string,
) {
	for (const id of pendingTimers) clearTimeout(id);
	pendingTimers.clear();

	window.__focusStepTarget = stepId;
	window.dispatchEvent(new CustomEvent("focus-step", { detail: stepId }));
	if (ownerRunId !== currentRunId) {
		window.dispatchEvent(
			new CustomEvent("focus-step-nested", { detail: { stepId } }),
		);
	}
	const tryScroll = (attempts: number) => {
		const el = document.getElementById(`step-${stepId}`);
		if (el) {
			el.scrollIntoView({ behavior: "smooth", block: "center" });
			const timer = setTimeout(() => {
				window.__focusStepTarget = null;
				pendingTimers.delete(timer);
			}, 2000);
			pendingTimers.add(timer);
		} else if (attempts > 0) {
			const timer = setTimeout(() => {
				pendingTimers.delete(timer);
				tryScroll(attempts - 1);
			}, 200);
			pendingTimers.add(timer);
		}
	};
	tryScroll(10);
}
