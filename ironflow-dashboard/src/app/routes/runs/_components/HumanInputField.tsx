import type { JSONSchema7 } from "json-schema";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { FieldWrapper } from "@/app/components/SchemaField";
import { classifyHumanInputField } from "@/app/lib/json-schema";

interface HumanInputFieldProps {
	name: string;
	schema: JSONSchema7;
	value: unknown;
	onChange: (value: unknown) => void;
	required: boolean;
}

/** One form field for a human input answer, widget chosen from the schema. */
export function HumanInputField({
	name,
	schema,
	value,
	onChange,
	required,
}: HumanInputFieldProps) {
	const label =
		typeof schema.title === "string" ? schema.title : name.replace(/_/g, " ");
	const description =
		typeof schema.description === "string" ? schema.description : undefined;
	const classified = classifyHumanInputField(schema);
	const fieldId = `field-${name}`;

	if (classified.kind === "checkbox") {
		return (
			<div className="flex items-start gap-2">
				<Checkbox
					id={fieldId}
					checked={value === true}
					onCheckedChange={(checked) => onChange(checked === true)}
				/>
				<div className="space-y-0.5">
					<label
						htmlFor={fieldId}
						className="text-sm font-medium cursor-pointer"
					>
						{label}
						{required && (
							<span aria-hidden="true" className="text-destructive ml-0.5">
								*
							</span>
						)}
					</label>
					{description && (
						<p className="text-xs text-muted-foreground">{description}</p>
					)}
				</div>
			</div>
		);
	}

	if (classified.kind === "radio") {
		return (
			<FieldWrapper
				name={name}
				label={label}
				description={description}
				required={required}
			>
				<div
					className="flex flex-col gap-2"
					role="radiogroup"
					aria-label={label}
				>
					{classified.options.map((option) => (
						<label
							key={option.value}
							className="flex items-center gap-2 text-sm cursor-pointer"
						>
							<input
								type="radio"
								name={fieldId}
								value={option.value}
								checked={value === option.value}
								onChange={() => onChange(option.value)}
								className="accent-primary"
							/>
							{option.label}
						</label>
					))}
				</div>
			</FieldWrapper>
		);
	}

	if (classified.kind === "number") {
		return (
			<FieldWrapper
				name={name}
				label={label}
				description={description}
				required={required}
			>
				<Input
					id={fieldId}
					type="number"
					step={classified.integer ? "1" : "any"}
					min={classified.minimum}
					max={classified.maximum}
					value={value != null ? String(value) : ""}
					onChange={(e) => {
						const raw = e.target.value;
						if (raw === "") {
							onChange(undefined);
							return;
						}
						onChange(classified.integer ? parseInt(raw, 10) : parseFloat(raw));
					}}
				/>
			</FieldWrapper>
		);
	}

	return (
		<FieldWrapper
			name={name}
			label={label}
			description={description}
			required={required}
		>
			<Textarea
				id={fieldId}
				className="text-sm"
				rows={4}
				value={typeof value === "string" ? value : ""}
				onChange={(e) => onChange(e.target.value)}
			/>
		</FieldWrapper>
	);
}
