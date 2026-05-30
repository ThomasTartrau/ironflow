import { Button } from "@/components/ui/button";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

interface PaginationProps {
	currentPage: number;
	totalPages: number;
	total: number;
	onPageChange: (page: number) => void;
	perPage?: number;
	className?: string;
}

function getVisiblePages(
	current: number,
	total: number,
): (number | "ellipsis")[] {
	if (total <= 7) {
		return Array.from({ length: total }, (_, i) => i + 1);
	}

	if (current <= 3) {
		return [1, 2, 3, 4, "ellipsis", total];
	}

	if (current >= total - 2) {
		return [1, "ellipsis", total - 3, total - 2, total - 1, total];
	}

	return [1, "ellipsis", current - 1, current, current + 1, "ellipsis", total];
}

export function Pagination({
	currentPage,
	totalPages,
	total,
	onPageChange,
	perPage,
	className,
}: PaginationProps) {
	const startItem = perPage
		? Math.min((currentPage - 1) * perPage + 1, total)
		: 1;
	const endItem = perPage ? Math.min(currentPage * perPage, total) : total;

	if (totalPages <= 1) {
		return (
			<div className={cn("flex items-center justify-between", className)}>
				<p className="text-xs text-muted-foreground tabular-nums">
					{perPage
						? `Showing ${startItem}-${endItem} of ${total}`
						: `${total} result${total !== 1 ? "s" : ""}`}
				</p>
			</div>
		);
	}

	const pages = getVisiblePages(currentPage, totalPages);

	return (
		<div className={cn("flex items-center justify-between", className)}>
			<p className="text-xs text-muted-foreground tabular-nums">
				{perPage
					? `Showing ${startItem}-${endItem} of ${total}`
					: `Page ${currentPage} of ${totalPages} (${total} total)`}
			</p>
			<div className="flex items-center gap-1">
				<Button
					variant="outline"
					size="sm"
					className="size-8 p-0"
					onClick={() => onPageChange(currentPage - 1)}
					disabled={currentPage <= 1}
					aria-label="Previous page"
				>
					<ChevronLeft className="size-4" />
				</Button>

				{pages.map((page, index) =>
					page === "ellipsis" ? (
						<span
							key={`ellipsis-${index}`}
							aria-hidden="true"
							className="px-1 text-xs text-muted-foreground select-none"
						>
							&hellip;
						</span>
					) : (
						<Button
							key={page}
							variant={page === currentPage ? "default" : "outline"}
							size="sm"
							className="size-8 p-0 text-xs tabular-nums"
							onClick={() => onPageChange(page)}
							aria-label={`Go to page ${page}`}
							aria-current={page === currentPage ? "page" : undefined}
						>
							{page}
						</Button>
					),
				)}

				<Button
					variant="outline"
					size="sm"
					className="size-8 p-0"
					onClick={() => onPageChange(currentPage + 1)}
					disabled={currentPage >= totalPages}
					aria-label="Next page"
				>
					<ChevronRight className="size-4" />
				</Button>
			</div>
		</div>
	);
}
