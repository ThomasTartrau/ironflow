import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { ArrowLeft } from "lucide-react";

interface BackLinkProps {
	to: string;
	label: string;
}

export function BackLink({ to, label }: BackLinkProps) {
	return (
		<Link to={to} className="inline-block">
			<Button variant="outline" size="sm" className="gap-1.5">
				<ArrowLeft className="w-4 h-4" />
				{label}
			</Button>
		</Link>
	);
}
