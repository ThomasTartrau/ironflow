import { useState, useMemo } from "react";
import { X, ChevronDown } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";

export interface MultiSelectOption {
	value: string;
	label: string;
	description?: string;
}

interface MultiSelectProps {
	options: MultiSelectOption[];
	value: string[];
	onChange: (value: string[]) => void;
	placeholder?: string;
	className?: string;
}

export function MultiSelect({
	options,
	value,
	onChange,
	placeholder = "Select...",
	className,
}: MultiSelectProps) {
	const [open, setOpen] = useState(false);
	const [search, setSearch] = useState("");

	const filtered = useMemo(
		() =>
			options.filter(
				(opt) =>
					opt.label.toLowerCase().includes(search.toLowerCase()) ||
					opt.value.toLowerCase().includes(search.toLowerCase()),
			),
		[options, search],
	);

	function toggle(optionValue: string) {
		onChange(
			value.includes(optionValue)
				? value.filter((v) => v !== optionValue)
				: [...value, optionValue],
		);
	}

	function remove(optionValue: string) {
		onChange(value.filter((v) => v !== optionValue));
	}

	function toggleAll() {
		const allValues = options.map((o) => o.value);
		onChange(value.length === options.length ? [] : allValues);
	}

	const allSelected = options.length > 0 && value.length === options.length;

	const selectedLabels = value
		.map((v) => options.find((o) => o.value === v))
		.filter(Boolean) as MultiSelectOption[];

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger
				render={
					<button
						type="button"
						className={cn(
							"flex min-h-10 w-full items-center justify-between rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background",
							"focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
							className,
						)}
					/>
				}
			>
				<div className="flex flex-1 flex-wrap gap-1">
					{selectedLabels.length === 0 ? (
						<span className="text-muted-foreground">{placeholder}</span>
					) : (
						selectedLabels.map((opt) => (
							<Badge
								key={opt.value}
								variant="secondary"
								className="text-xs gap-1"
							>
								{opt.label}
								<button
									type="button"
									className="rounded-full outline-none hover:bg-muted-foreground/20"
									onPointerDown={(e) => {
										e.preventDefault();
										e.stopPropagation();
									}}
									onClick={(e) => {
										e.stopPropagation();
										remove(opt.value);
									}}
								>
									<X className="h-3 w-3" />
								</button>
							</Badge>
						))
					)}
				</div>
				<ChevronDown className="h-4 w-4 shrink-0 opacity-50" />
			</PopoverTrigger>
			<PopoverContent
				className="w-(--anchor-width) p-0"
				align="start"
			>
				<div className="p-2">
					<input
						type="text"
						className="w-full rounded-md border border-input bg-transparent px-3 py-1.5 text-sm outline-none placeholder:text-muted-foreground"
						placeholder="Search..."
						value={search}
						onChange={(e) => setSearch(e.target.value)}
					/>
				</div>
				<div className="max-h-60 overflow-y-auto px-1 pb-1">
					{!search && options.length > 1 && (
						<>
							<button
								type="button"
								className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-accent"
								onClick={toggleAll}
							>
								<Checkbox
									checked={allSelected}
									className="pointer-events-none"
									tabIndex={-1}
								/>
								<span className="text-sm font-medium">
									{allSelected ? "Deselect all" : "Select all"}
								</span>
							</button>
							<div className="mx-2 my-1 border-t border-border" />
						</>
					)}
					{filtered.length === 0 ? (
						<p className="py-4 text-center text-sm text-muted-foreground">
							No results.
						</p>
					) : (
						filtered.map((opt) => (
							<button
								type="button"
								key={opt.value}
								className="flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left hover:bg-accent"
								onClick={() => toggle(opt.value)}
							>
								<Checkbox
									checked={value.includes(opt.value)}
									className="mt-0.5 pointer-events-none"
									tabIndex={-1}
								/>
								<div className="grid gap-0.5">
									<span className="text-sm font-medium">
										{opt.label}
									</span>
									{opt.description && (
										<span className="text-xs text-muted-foreground">
											{opt.description}
										</span>
									)}
								</div>
							</button>
						))
					)}
				</div>
			</PopoverContent>
		</Popover>
	);
}
