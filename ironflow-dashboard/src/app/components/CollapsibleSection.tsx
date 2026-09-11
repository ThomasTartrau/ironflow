import type { ReactNode } from "react";
import { usePersistedOpen } from "@/app/hooks/use-persisted-open";
import { ChevronRight } from "lucide-react";

interface CollapsibleSectionProps {
	storageKey: string;
	title: string;
	defaultOpen?: boolean;
	actions?: ReactNode;
	accent?: string;
	children: ReactNode;
}

export function CollapsibleSection({
	storageKey,
	title,
	defaultOpen = false,
	actions,
	accent,
	children,
}: CollapsibleSectionProps) {
	const [open, toggle] = usePersistedOpen(storageKey, defaultOpen);
	const triggerId = `collapsible-trigger-${storageKey}`;
	const contentId = `collapsible-content-${storageKey}`;

	return (
		<div>
			<div className="flex items-center justify-between mb-3">
				<button
					id={triggerId}
					type="button"
					onClick={toggle}
					aria-expanded={open}
					aria-controls={contentId}
					className="flex items-center gap-2 py-1 group"
				>
					{accent && (
						<div
							className="w-0.5 h-4 rounded-full shrink-0 transition-opacity duration-200"
							style={{
								backgroundColor: accent,
								opacity: open ? 1 : 0.35,
							}}
						/>
					)}
					<ChevronRight
						aria-hidden
						className={`h-3.5 w-3.5 text-muted-foreground/60 transition-transform duration-200 ease-out ${open ? "rotate-90" : ""}`}
					/>
					<span className="text-sm font-medium text-foreground/70 group-hover:text-foreground transition-colors">
						{title}
					</span>
				</button>
				{actions}
			</div>
			<section
				id={contentId}
				aria-labelledby={triggerId}
				className="grid transition-[grid-template-rows] duration-300 ease-out motion-reduce:transition-none"
				style={{ gridTemplateRows: open ? "1fr" : "0fr" }}
			>
				<div className="overflow-hidden">
					<div
						className="transition-opacity duration-200 motion-reduce:transition-none"
						style={{
							opacity: open ? 1 : 0,
							transitionDelay: open ? "100ms" : "0ms",
						}}
					>
						{children}
					</div>
				</div>
			</section>
		</div>
	);
}
