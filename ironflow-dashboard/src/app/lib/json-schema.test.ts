import { describe, expect, it } from "vitest";
import type { JSONSchema7 } from "json-schema";
import { validateAgainstSchema } from "./json-schema";

const answersSchema: JSONSchema7 = {
	type: "object",
	required: ["answers"],
	properties: {
		answers: { type: "array", items: { type: "string" } },
		note: { type: ["string", "null"] },
	},
};

describe("validateAgainstSchema", () => {
	it("accepts a value matching the schema", () => {
		const value = { answers: ["yes", "no"], note: null };
		expect(validateAgainstSchema(value, answersSchema)).toEqual([]);
	});

	it("reports a missing required property", () => {
		expect(validateAgainstSchema({}, answersSchema)).toEqual([
			'(root): missing required property "answers"',
		]);
	});

	it("reports a wrong type with its path", () => {
		expect(validateAgainstSchema({ answers: 3 }, answersSchema)).toEqual([
			"/answers: expected array, got integer",
		]);
		expect(validateAgainstSchema([], answersSchema)).toEqual([
			"(root): expected object, got array",
		]);
	});

	it("checks every array item", () => {
		expect(
			validateAgainstSchema({ answers: ["ok", 1, true] }, answersSchema),
		).toEqual([
			"/answers/1: expected string, got integer",
			"/answers/2: expected string, got boolean",
		]);
	});

	it("resolves $ref against $defs", () => {
		const schema = {
			type: "object",
			required: ["target"],
			properties: { target: { $ref: "#/$defs/Target" } },
			$defs: {
				Target: {
					type: "object",
					required: ["env"],
					properties: { env: { type: "string" } },
				},
			},
		} as JSONSchema7;

		expect(validateAgainstSchema({ target: { env: "prod" } }, schema)).toEqual(
			[],
		);
		expect(validateAgainstSchema({ target: {} }, schema)).toEqual([
			'/target: missing required property "env"',
		]);
	});

	it("reports an unresolved reference", () => {
		const schema: JSONSchema7 = { $ref: "#/$defs/Missing" };
		expect(validateAgainstSchema({}, schema)).toEqual([
			"(root): unresolved reference #/$defs/Missing",
		]);
	});

	it("checks enum values", () => {
		const schema: JSONSchema7 = {
			type: "string",
			enum: ["staging", "production"],
		};
		expect(validateAgainstSchema("staging", schema)).toEqual([]);
		expect(validateAgainstSchema("dev", schema)).toEqual([
			'(root): must be one of "staging", "production"',
		]);
	});

	it("treats an integer as a number", () => {
		expect(validateAgainstSchema(3, { type: "number" })).toEqual([]);
		expect(validateAgainstSchema(1.5, { type: "integer" })).toEqual([
			"(root): expected integer, got number",
		]);
	});
});
