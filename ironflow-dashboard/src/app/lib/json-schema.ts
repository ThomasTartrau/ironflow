import type { JSONSchema7, JSONSchema7Definition } from "json-schema";

export function isJsonSchema(value: unknown): value is JSONSchema7 {
	return (
		typeof value === "object" &&
		value !== null &&
		"type" in value &&
		(value as JSONSchema7).type === "object"
	);
}

function resolveProperty(
	def: JSONSchema7Definition,
): JSONSchema7 | null {
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
