import type { JSONSchema7, JSONSchema7Definition } from "json-schema";

export function isJsonSchema(value: unknown): value is JSONSchema7 {
	return (
		typeof value === "object" &&
		value !== null &&
		"type" in value &&
		(value as JSONSchema7).type === "object"
	);
}

function resolveProperty(def: JSONSchema7Definition): JSONSchema7 | null {
	if (typeof def === "boolean") return null;
	return def;
}

export function extractSchemaProperties(schema: JSONSchema7 | null): {
	properties: Record<string, JSONSchema7>;
	requiredFields: Set<string>;
} {
	const rawProperties = schema?.properties ?? {};
	const requiredFields = new Set(schema?.required ?? []);

	const properties: Record<string, JSONSchema7> = {};
	for (const [key, def] of Object.entries(rawProperties)) {
		const resolved = resolveProperty(def);
		if (resolved) properties[key] = resolved;
	}

	return { properties, requiredFields };
}

export function buildDefaultValues(
	properties: Record<string, JSONSchema7>,
): Record<string, unknown> {
	const initial: Record<string, unknown> = {};
	for (const [key, prop] of Object.entries(properties)) {
		if (prop.default != null) {
			initial[key] = prop.default;
		}
	}
	return initial;
}

export interface RadioOption {
	value: string;
	label: string;
}

export type HumanInputFieldKind =
	| { kind: "textarea" }
	| { kind: "radio"; options: RadioOption[] }
	| { kind: "checkbox" }
	| { kind: "number"; integer: boolean; minimum?: number; maximum?: number }
	| { kind: "unsupported" };

function extractRadioOptions(schema: JSONSchema7): RadioOption[] | null {
	if (
		Array.isArray(schema.enum) &&
		schema.enum.every((v) => typeof v === "string")
	) {
		return schema.enum.map((v) => ({ value: v as string, label: v as string }));
	}
	if (Array.isArray(schema.oneOf)) {
		const options: RadioOption[] = [];
		for (const sub of schema.oneOf) {
			if (typeof sub === "boolean" || typeof sub.const !== "string")
				return null;
			options.push({
				value: sub.const,
				label: typeof sub.title === "string" ? sub.title : sub.const,
			});
		}
		return options.length > 0 ? options : null;
	}
	return null;
}

/** Which widget `HumanInputField` should render for a top-level property schema. */
export function classifyHumanInputField(
	schema: JSONSchema7,
): HumanInputFieldKind {
	if (schema.type === "boolean") return { kind: "checkbox" };
	if (schema.type === "integer" || schema.type === "number") {
		return {
			kind: "number",
			integer: schema.type === "integer",
			minimum: schema.minimum,
			maximum: schema.maximum,
		};
	}
	if (schema.type === "string") {
		const options = extractRadioOptions(schema);
		return options ? { kind: "radio", options } : { kind: "textarea" };
	}
	return { kind: "unsupported" };
}

/** Whether every top-level property can be rendered as a field (no nesting, arrays, or unsupported unions). */
export function canRenderForm(
	properties: Record<string, JSONSchema7>,
): boolean {
	return Object.values(properties).every(
		(p) => classifyHumanInputField(p).kind !== "unsupported",
	);
}

function emptySkeletonValue(prop: JSONSchema7 | undefined): unknown {
	switch (prop?.type) {
		case "boolean":
			return false;
		case "integer":
		case "number":
			return 0;
		case "array":
			return [];
		case "object":
			return {};
		default:
			return "";
	}
}

/** Skeleton JSON of the required properties, e.g. `{"reply": ""}` instead of `{}`, for the JSON-editor fallback. */
export function buildAnswerSkeleton(
	schema: JSONSchema7 | null,
): Record<string, unknown> {
	const { properties, requiredFields } = extractSchemaProperties(schema);
	const skeleton: Record<string, unknown> = {};
	for (const key of requiredFields) {
		const prop = properties[key];
		skeleton[key] =
			prop?.default !== undefined ? prop.default : emptySkeletonValue(prop);
	}
	return skeleton;
}

type JsonType =
	| "object"
	| "array"
	| "string"
	| "number"
	| "integer"
	| "boolean"
	| "null";

function jsonTypeOf(value: unknown): JsonType {
	if (value === null) return "null";
	if (Array.isArray(value)) return "array";
	if (typeof value === "number") {
		return Number.isInteger(value) ? "integer" : "number";
	}
	if (typeof value === "string") return "string";
	if (typeof value === "boolean") return "boolean";
	return "object";
}

function matchesType(value: unknown, expected: string): boolean {
	const actual = jsonTypeOf(value);
	if (expected === "number") return actual === "number" || actual === "integer";
	return actual === expected;
}

function resolveRef(
	ref: string,
	root: JSONSchema7,
): JSONSchema7Definition | null {
	const prefix = "#/$defs/";
	if (!ref.startsWith(prefix)) return null;
	const defs = (root as { $defs?: Record<string, JSONSchema7Definition> })
		.$defs;
	return defs?.[ref.slice(prefix.length)] ?? null;
}

function displayPath(path: string): string {
	return path === "" ? "(root)" : path;
}

function validateNode(
	value: unknown,
	def: JSONSchema7Definition,
	root: JSONSchema7,
	path: string,
	errors: string[],
): void {
	if (def === true) return;
	if (def === false) {
		errors.push(`${displayPath(path)}: no value is allowed here`);
		return;
	}

	if (def.$ref) {
		const target = resolveRef(def.$ref, root);
		if (target === null) {
			errors.push(`${displayPath(path)}: unresolved reference ${def.$ref}`);
			return;
		}
		validateNode(value, target, root, path, errors);
		return;
	}

	if (def.type !== undefined) {
		const expected = Array.isArray(def.type) ? def.type : [def.type];
		if (!expected.some((t) => matchesType(value, t))) {
			const wanted = expected.join(" or ");
			const actual = jsonTypeOf(value);
			errors.push(`${displayPath(path)}: expected ${wanted}, got ${actual}`);
			return;
		}
	}

	if (def.enum !== undefined) {
		const encoded = JSON.stringify(value);
		const allowed = def.enum.map((candidate) => JSON.stringify(candidate));
		if (!allowed.includes(encoded)) {
			errors.push(`${displayPath(path)}: must be one of ${allowed.join(", ")}`);
		}
	}

	if (jsonTypeOf(value) === "object") {
		const obj = value as Record<string, unknown>;
		for (const key of def.required ?? []) {
			if (!(key in obj)) {
				errors.push(`${displayPath(path)}: missing required property "${key}"`);
			}
		}
		for (const [key, propDef] of Object.entries(def.properties ?? {})) {
			if (key in obj) {
				validateNode(obj[key], propDef, root, `${path}/${key}`, errors);
			}
		}
	}

	if (Array.isArray(value) && def.items !== undefined) {
		if (Array.isArray(def.items)) {
			def.items.forEach((itemDef, index) => {
				if (index < value.length) {
					validateNode(value[index], itemDef, root, `${path}/${index}`, errors);
				}
			});
		} else {
			const itemDef = def.items;
			value.forEach((item, index) => {
				validateNode(item, itemDef, root, `${path}/${index}`, errors);
			});
		}
	}
}

/**
 * Check `value` against a JSON schema and list every violation found.
 *
 * A minimal client-side check covering `type`, `required`, `properties`,
 * `items`, `enum` and `$ref` to `#/$defs/*`, enough for the schemas the
 * engine derives from Rust types. Each error starts with a JSON-pointer-like
 * path, `(root)` for the value itself. The server stays authoritative: it
 * validates the full schema and answers 422 on a mismatch.
 */
export function validateAgainstSchema(
	value: unknown,
	schema: JSONSchema7,
): string[] {
	const errors: string[] = [];
	validateNode(value, schema, schema, "", errors);
	return errors;
}
