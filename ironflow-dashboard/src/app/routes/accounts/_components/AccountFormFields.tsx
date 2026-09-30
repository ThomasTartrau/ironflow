import type { ProviderAccountResponse } from "@/app/lib/types";
import { Input } from "@/components/ui/input";
import {
	InputGroup,
	InputGroupAddon,
	InputGroupInput,
	InputGroupText,
} from "@/components/ui/input-group";
import { percentToThreshold, thresholdToPercent } from "./account-state";

/** Raw input values of the account form, shared by the add and edit forms. */
export interface AccountFormValues {
	name: string;
	displayName: string;
	token: string;
	tags: string;
	priority: string;
	maxConcurrency: string;
	threshold: string;
}

/** Initial values of the add form. */
export const EMPTY_ACCOUNT_FORM: AccountFormValues = {
	name: "",
	displayName: "",
	token: "",
	tags: "",
	priority: "100",
	maxConcurrency: "",
	threshold: "80",
};

/** Initial values of the edit form; the token stays empty (write-only). */
export function accountFormValues(
	account: ProviderAccountResponse,
): AccountFormValues {
	return {
		name: account.name,
		displayName: account.display_name,
		token: "",
		tags: account.tags.join(", "),
		priority: String(account.priority),
		maxConcurrency:
			account.max_concurrency == null ? "" : String(account.max_concurrency),
		threshold: thresholdToPercent(account.alert_threshold),
	};
}

/** Fields both requests share, parsed from the raw inputs. */
export function parseAccountForm(values: AccountFormValues) {
	return {
		tags: values.tags
			.split(",")
			.map((t) => t.trim())
			.filter(Boolean),
		priority: Number(values.priority),
		max_concurrency:
			values.maxConcurrency === "" ? null : Number(values.maxConcurrency),
		alert_threshold: percentToThreshold(values.threshold),
	};
}

interface FieldProps {
	id: string;
	label: string;
	hint: string;
	optional?: boolean;
	children: React.ReactNode;
}

function Field({ id, label, hint, optional, children }: FieldProps) {
	return (
		<div className="space-y-1">
			<label htmlFor={id} className="text-sm font-medium">
				{label}
				{optional && (
					<span className="ml-1 font-normal text-muted-foreground">
						(optional)
					</span>
				)}
			</label>
			{children}
			<p className="text-xs text-muted-foreground">{hint}</p>
		</div>
	);
}

interface AccountFormFieldsProps {
	/** `create` shows the identifier and requires the token. */
	mode: "create" | "edit";
	/** Prefix of the input ids, unique on the page. */
	idPrefix: string;
	values: AccountFormValues;
	onChange: (values: AccountFormValues) => void;
	tokenLabel?: string;
	tokenHint?: string;
}

/** Inputs of a Provider Account, one per row, with a label and a help line. */
export function AccountFormFields({
	mode,
	idPrefix,
	values,
	onChange,
	tokenLabel = "Token",
	tokenHint = "Run `claude setup-token` and paste the sk-ant-oat01-... token.",
}: AccountFormFieldsProps) {
	const creating = mode === "create";
	const id = (field: string) => `${idPrefix}-${field}`;
	const set =
		(field: keyof AccountFormValues) =>
		(e: React.ChangeEvent<HTMLInputElement>) =>
			onChange({ ...values, [field]: e.target.value });

	return (
		<div className="space-y-4">
			{creating && (
				<Field
					id={id("name")}
					label="Identifier"
					hint="Unique identifier used by the CLI and in logs. Lowercase letters, digits and hyphens. Cannot be changed later."
				>
					<Input
						id={id("name")}
						placeholder="e.g. perso-max"
						required
						pattern="[a-z0-9][a-z0-9-]{0,62}"
						className="font-mono"
						autoComplete="off"
						spellCheck={false}
						value={values.name}
						onChange={set("name")}
					/>
				</Field>
			)}
			<Field
				id={id("display-name")}
				label="Display name"
				optional={creating}
				hint="Name shown in the dashboard. Defaults to the identifier."
			>
				<Input
					id={id("display-name")}
					placeholder="e.g. Perso Max"
					required={!creating}
					value={values.displayName}
					onChange={set("displayName")}
				/>
			</Field>
			<Field
				id={id("token")}
				label={creating ? tokenLabel : "New token"}
				optional={!creating}
				hint={
					creating
						? tokenHint
						: "Leave empty to keep the current token. A new token is checked before it is saved."
				}
			>
				<Input
					id={id("token")}
					type="password"
					autoComplete="off"
					required={creating}
					placeholder="sk-ant-oat01-..."
					className="font-mono"
					value={values.token}
					onChange={set("token")}
				/>
			</Field>
			<Field
				id={id("tags")}
				label="Tags"
				optional
				hint="Free labels to organize your accounts, comma separated."
			>
				<Input
					id={id("tags")}
					placeholder="e.g. perso, team"
					value={values.tags}
					onChange={set("tags")}
				/>
			</Field>
			<Field
				id={id("priority")}
				label="Priority"
				hint="Lower is preferred. When several accounts have the same remaining usage, the one with the lowest priority is used first."
			>
				<Input
					id={id("priority")}
					type="number"
					required
					value={values.priority}
					onChange={set("priority")}
				/>
			</Field>
			<Field
				id={id("max-concurrency")}
				label="Max concurrent steps"
				optional
				hint="Maximum number of agent steps running at the same time on this account. Leave empty for no limit."
			>
				<Input
					id={id("max-concurrency")}
					type="number"
					min={1}
					placeholder="No limit"
					value={values.maxConcurrency}
					onChange={set("maxConcurrency")}
				/>
			</Field>
			<Field
				id={id("threshold")}
				label="Alert threshold"
				hint="Usage level from which the account is flagged as Near limit."
			>
				<InputGroup>
					<InputGroupInput
						id={id("threshold")}
						type="number"
						required
						step={5}
						min={5}
						max={100}
						value={values.threshold}
						onChange={set("threshold")}
					/>
					<InputGroupAddon align="inline-end">
						<InputGroupText>%</InputGroupText>
					</InputGroupAddon>
				</InputGroup>
			</Field>
		</div>
	);
}
