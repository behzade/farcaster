import { Compile, type Validator } from "typebox/compile";
import type { TSchema } from "typebox";
import { ProjectToolLoadError } from "./errors.ts";

const GENERAL_KEYS = new Set(["type", "title", "description", "default", "enum", "const", "anyOf", "oneOf"]);
const KEYS_BY_TYPE: Readonly<Record<string, ReadonlySet<string>>> = {
  object: new Set(["properties", "required", "additionalProperties", "minProperties", "maxProperties"]),
  array: new Set(["items", "minItems", "maxItems", "uniqueItems"]),
  string: new Set(["minLength", "maxLength", "pattern"]),
  number: new Set(["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf"]),
  integer: new Set(["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf"]),
  boolean: new Set(),
  null: new Set(),
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isJsonValue = (value: unknown): boolean => {
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(isJsonValue);
  return isRecord(value) && Object.values(value).every(isJsonValue);
};

function fail(path: string, message: string): never {
  throw new ProjectToolLoadError({ path, message });
}
function checkFiniteNumber(value: unknown, path: string): void {
  if (value !== undefined && (typeof value !== "number" || !Number.isFinite(value))) {
    fail(path, "must be a finite number");
  }
}

function checkCount(value: unknown, path: string): void {
  if (value !== undefined && (!Number.isInteger(value) || (value as number) < 0)) {
    fail(path, "must be a non-negative integer");
  }
}

function validateSchemaNode(value: unknown, path: string, depth: number): asserts value is TSchema {
  if (depth > 32) fail(path, "schema nesting exceeds 32 levels");
  if (!isRecord(value)) fail(path, "must be a JSON Schema object");

  for (const blocked of ["$ref", "$dynamicRef", "$recursiveRef", "$id", "$schema", "format", "patternProperties"] as const) {
    if (blocked in value) fail(`${path}.${blocked}`, "is not supported");
  }

  const alternatives = value.anyOf ?? value.oneOf;
  if (value.anyOf !== undefined && value.oneOf !== undefined) {
    fail(path, "cannot contain both anyOf and oneOf");
  }
  if (alternatives !== undefined) {
    if (!Array.isArray(alternatives) || alternatives.length < 1 || alternatives.length > 16) {
      fail(`${path}.${value.anyOf !== undefined ? "anyOf" : "oneOf"}`, "must contain 1 to 16 schemas");
    }
    alternatives.forEach((schema, index) => validateSchemaNode(schema, `${path}.${value.anyOf !== undefined ? "anyOf" : "oneOf"}[${index}]`, depth + 1));
  }

  const type = value.type;
  if (typeof type !== "string" || !(type in KEYS_BY_TYPE)) {
    fail(`${path}.type`, "must be one supported JSON Schema type");
  }

  const allowed = new Set([...GENERAL_KEYS, ...KEYS_BY_TYPE[type]!]);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) fail(`${path}.${key}`, "is not supported");
  }

  if (value.title !== undefined && typeof value.title !== "string") fail(`${path}.title`, "must be a string");
  if (value.description !== undefined && typeof value.description !== "string") fail(`${path}.description`, "must be a string");
  if (value.default !== undefined && !isJsonValue(value.default)) fail(`${path}.default`, "must be JSON data");
  if (value.const !== undefined && !isJsonValue(value.const)) fail(`${path}.const`, "must be JSON data");
  if (value.enum !== undefined && (!Array.isArray(value.enum) || value.enum.length === 0 || !value.enum.every(isJsonValue))) {
    fail(`${path}.enum`, "must be a non-empty array of JSON values");
  }

  if (type === "object") {
    if (value.additionalProperties !== false) fail(`${path}.additionalProperties`, "must be false");
    if (!isRecord(value.properties)) fail(`${path}.properties`, "must be an object");
    for (const [name, schema] of Object.entries(value.properties)) {
      validateSchemaNode(schema, `${path}.properties.${name}`, depth + 1);
    }
    if (!Array.isArray(value.required) || !value.required.every((item) => typeof item === "string")) {
      fail(`${path}.required`, "must be an array of property names");
    }
    const propertyNames = new Set(Object.keys(value.properties));
    const required = value.required as string[];
    if (new Set(required).size !== required.length || required.some((name) => !propertyNames.has(name))) {
      fail(`${path}.required`, "must contain unique declared property names");
    }
    checkCount(value.minProperties, `${path}.minProperties`);
    checkCount(value.maxProperties, `${path}.maxProperties`);
    if (typeof value.minProperties === "number" && typeof value.maxProperties === "number" && value.minProperties > value.maxProperties) {
      fail(path, "minProperties must not exceed maxProperties");
    }
  } else if (type === "array") {
    validateSchemaNode(value.items, `${path}.items`, depth + 1);
    checkCount(value.minItems, `${path}.minItems`);
    checkCount(value.maxItems, `${path}.maxItems`);
    if (typeof value.minItems === "number" && typeof value.maxItems === "number" && value.minItems > value.maxItems) {
      fail(path, "minItems must not exceed maxItems");
    }
    if (value.uniqueItems !== undefined && typeof value.uniqueItems !== "boolean") {
      fail(`${path}.uniqueItems`, "must be a boolean");
    }
  } else if (type === "string") {
    checkCount(value.minLength, `${path}.minLength`);
    checkCount(value.maxLength, `${path}.maxLength`);
    if (typeof value.minLength === "number" && typeof value.maxLength === "number" && value.minLength > value.maxLength) {
      fail(path, "minLength must not exceed maxLength");
    }
    if (value.pattern !== undefined && typeof value.pattern !== "string") fail(`${path}.pattern`, "must be a string");
    if (typeof value.pattern === "string") {
      try {
        new RegExp(value.pattern);
      } catch (cause) {
        throw new ProjectToolLoadError({ path: `${path}.pattern`, message: "must be a valid regular expression", cause });
      }
    }
  } else if (type === "number" || type === "integer") {
    for (const key of ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf"] as const) {
      checkFiniteNumber(value[key], `${path}.${key}`);
    }
    if (typeof value.multipleOf === "number" && value.multipleOf <= 0) fail(`${path}.multipleOf`, "must be greater than zero");
    if (typeof value.minimum === "number" && typeof value.maximum === "number" && value.minimum > value.maximum) {
      fail(path, "minimum must not exceed maximum");
    }
  }
}

export function compileStrictSchema(value: unknown, path: string, requireObjectRoot = false): Validator {
  validateSchemaNode(value, path, 0);
  if (requireObjectRoot && value.type !== "object") fail(`${path}.type`, "must be object");
  try {
    return Compile(value);
  } catch (cause) {
    throw new ProjectToolLoadError({ path, message: "could not compile schema", cause });
  }
}

export function validationMessage(validator: Validator, value: unknown): string {
  const messages = [...validator.Errors(value)].slice(0, 5).map((error) => {
    const path = error.instancePath ? error.instancePath.replace(/^\//, "").replaceAll("/", ".") : "root";
    return `${path}: ${error.message}`;
  });
  return messages.join("; ") || "value does not match schema";
}
