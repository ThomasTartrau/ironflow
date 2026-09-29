import { describe, expect, it } from "vitest";
import type { JSONSchema7 } from "json-schema";
import {
	validateAgainstSchema,
	classifyHumanInputField,
	canRenderForm,
	buildAnswerSkeleton,
} from "./json-schema";

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

describe("classifyHumanInputField", () => {
	it("classifies a plain string as textarea", () => {
		expect(classifyHumanInputField({ type: "string" })).toEqual({
			kind: "textarea",
		});
	});

	it("classifies a string with enum as radio", () => {
		expect(
			classifyHumanInputField({
				type: "string",
				enum: ["staging", "production"],
			}),
		).toEqual({
			kind: "radio",
			options: [
				{ value: "staging", label: "staging" },
				{ value: "production", label: "production" },
			],
		});
	});

	it("classifies a string with oneOf of const+title as radio", () => {
		expect(
			classifyHumanInputField({
				type: "string",
				oneOf: [
					{ const: "staging", title: "Staging" },
					{ const: "production", title: "Production" },
				],
			}),
		).toEqual({
			kind: "radio",
			options: [
				{ value: "staging", label: "Staging" },
				{ value: "production", label: "Production" },
			],
		});
	});

	it("classifies a boolean as checkbox", () => {
		expect(classifyHumanInputField({ type: "boolean" })).toEqual({
			kind: "checkbox",
		});
	});

	it("carries minimum/maximum through for an integer", () => {
		expect(
			classifyHumanInputField({ type: "integer", minimum: 0, maximum: 10 }),
		).toEqual({ kind: "number", integer: true, minimum: 0, maximum: 10 });
	});

	it("carries minimum/maximum through for a number", () => {
		expect(classifyHumanInputField({ type: "number", minimum: 0.5 })).toEqual({
			kind: "number",
			integer: false,
			minimum: 0.5,
			maximum: undefined,
		});
	});

	it("classifies an array as unsupported", () => {
		expect(classifyHumanInputField({ type: "array" })).toEqual({
			kind: "unsupported",
		});
	});

	it("classifies an object as unsupported", () => {
		expect(classifyHumanInputField({ type: "object" })).toEqual({
			kind: "unsupported",
		});
	});
});

describe("canRenderForm", () => {
	it("is true when every property is covered", () => {
		expect(
			canRenderForm({
				reply: { type: "string" },
				count: { type: "integer" },
				agree: { type: "boolean" },
			}),
		).toBe(true);
	});

	it("is false as soon as one property is an array", () => {
		expect(
			canRenderForm({
				reply: { type: "string" },
				answers: { type: "array" },
			}),
		).toBe(false);
	});
});

describe("buildAnswerSkeleton", () => {
	it("returns an empty string for a required string", () => {
		expect(
			buildAnswerSkeleton({
				type: "object",
				required: ["reply"],
				properties: { reply: { type: "string" } },
			}),
		).toEqual({ reply: "" });
	});

	it("returns zero for a required integer", () => {
		expect(
			buildAnswerSkeleton({
				type: "object",
				required: ["count"],
				properties: { count: { type: "integer" } },
			}),
		).toEqual({ count: 0 });
	});

	it("uses the property's default instead of the empty value", () => {
		expect(
			buildAnswerSkeleton({
				type: "object",
				required: ["reply"],
				properties: { reply: { type: "string", default: "hello" } },
			}),
		).toEqual({ reply: "hello" });
	});

	it("only includes properties listed in required", () => {
		expect(
			buildAnswerSkeleton({
				type: "object",
				required: ["reply"],
				properties: {
					reply: { type: "string" },
					note: { type: "string" },
				},
			}),
		).toEqual({ reply: "" });
	});
});
