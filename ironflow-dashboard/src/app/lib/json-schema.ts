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
