import type { JSONSchema7 } from "json-schema";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";

interface SchemaFieldProps {
	name: string;
	schema: JSONSchema7;
	value: unknown;
	onChange: (value: unknown) => void;
	required: boolean;
}

export function SchemaField({
	name,
	schema,
	value,
	onChange,
	required,
}: SchemaFieldProps) {
	const label = name.replace(/_/g, " ");

	if (schema.type === "boolean") {
		return (
			<div className="flex items-center justify-between gap-3">
				<div>
					<p className="text-sm font-medium">{label}</p>
					{schema.description && (
						<p className="text-xs text-muted-foreground">
							{schema.description}
						</p>
					)}
				</div>
				<Switch
					checked={value === true}
					onCheckedChange={(checked) => onChange(checked)}
				/>
			</div>
		);
	}

	if (schema.type === "number" || schema.type === "integer") {
		return (
			<div className="space-y-1.5">
				<label className="text-sm font-medium" htmlFor={`field-${name}`}>
					{label}
					{required && <span className="text-destructive ml-0.5">*</span>}
				</label>
				{schema.description && (
					<p className="text-xs text-muted-foreground">{schema.description}</p>
				)}
				<Input
					id={`field-${name}`}
					type="number"
					step={schema.type === "integer" ? "1" : "any"}
					value={value != null ? String(value) : ""}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") {
							onChange(undefined);
							return;
						}
						onChange(
							schema.type === "integer" ? parseInt(raw, 10) : parseFloat(raw),
						);
					}}
				/>
			</div>
		);
	}

	return (
		<div className="space-y-1.5">
			<label className="text-sm font-medium" htmlFor={`field-${name}`}>
				{label}
				{required && <span className="text-destructive ml-0.5">*</span>}
			</label>
			{schema.description && (
				<p className="text-xs text-muted-foreground">{schema.description}</p>
			)}
			<Input
				id={`field-${name}`}
				type="text"
				value={typeof value === "string" ? value : ""}
				onChange={(e) => onChange(e.target.value)}
				placeholder={
					schema.default != null ? String(schema.default) : undefined
				}
			/>
		</div>
	);
}
