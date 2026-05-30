import { cn } from "@/lib/utils";

interface HeaderAppProps {
	title: string;
	description: string;
	titleItem?: React.ReactNode;
	children?: React.ReactNode;
	className?: string;
}

export function HeaderApp({
	title,
	description,
	titleItem,
	children,
	className,
}: HeaderAppProps) {
	return (
		<div className={cn("px-4 py-2", className)}>
			<div className="mb-4">
				<div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-2">
					<h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
					{titleItem && <div className="w-full sm:w-fit">{titleItem}</div>}
				</div>
				<p className="mt-1 text-sm text-muted-foreground">{description}</p>
			</div>
			{children}
		</div>
	);
}
