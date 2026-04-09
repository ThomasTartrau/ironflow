import type { ReactNode } from "react";
import { usePersistedOpen } from "@/app/hooks/use-persisted-open";
import { Button } from "@/components/ui/button";
import { ChevronRight } from "lucide-react";

interface CollapsibleSectionProps {
	storageKey: string;
	title: string;
	defaultOpen?: boolean;
	children: ReactNode;
}

export function CollapsibleSection({
	storageKey,
	title,
	defaultOpen = false,
	children,
}: CollapsibleSectionProps) {
	const [open, toggle] = usePersistedOpen(storageKey, defaultOpen);

	return (
		<div>
			<Button
				variant="ghost"
				size="sm"
				onClick={toggle}
				className="gap-1.5 text-foreground/60 hover:text-foreground/80 mb-2 px-2"
			>
				<ChevronRight
					className={`h-3.5 w-3.5 transition-transform ${open ? "rotate-90" : ""}`}
				/>
				{title}
			</Button>
			{open && children}
		</div>
	);
}
