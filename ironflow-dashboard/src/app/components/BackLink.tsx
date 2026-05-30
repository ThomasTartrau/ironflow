import { Link } from "react-router";
import { buttonVariants } from "@/components/ui/button";
import { ArrowLeft } from "lucide-react";
import { cn } from "@/lib/utils";

interface BackLinkProps {
	to: string;
	label: string;
}

export function BackLink({ to, label }: BackLinkProps) {
	return (
		<Link
			to={to}
			className={cn(
				buttonVariants({ variant: "ghost", size: "sm" }),
				"gap-1.5 text-muted-foreground hover:text-foreground",
			)}
		>
			<ArrowLeft className="size-4" />
			{label}
		</Link>
	);
}
