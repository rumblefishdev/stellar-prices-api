/**
 * The slice of OpenAPI 3.1 the API reference renders, and the helpers that
 * turn it into labels and examples.
 *
 * Not a general OpenAPI model. The document is `utoipa`'s output for THIS
 * API — `packages/prices-api/src/openapi/mod.rs` — and it uses a small,
 * regular subset: `$ref`s into `components.schemas`, objects with
 * `properties`, enums, arrays, `type: [T, "null"]` and `oneOf: [null, $ref]`
 * for optional fields. Everything below is written for that shape and says
 * `any` when it meets something else, rather than guessing. When the handler
 * starts emitting a construct this cannot label, the label shows it.
 */

export type Schema = {
  $ref?: string;
  type?: string | string[];
  format?: string;
  description?: string;
  enum?: (string | number)[];
  items?: Schema;
  properties?: Record<string, Schema>;
  required?: string[];
  oneOf?: Schema[];
  anyOf?: Schema[];
  allOf?: Schema[];
  example?: unknown;
  default?: unknown;
  minimum?: number;
  maximum?: number;
  minItems?: number;
  maxItems?: number;
};

export type Parameter = {
  name: string;
  in: 'query' | 'path' | 'header' | 'cookie';
  description?: string;
  required?: boolean;
  deprecated?: boolean;
  schema?: Schema;
};

export type MediaType = { schema?: Schema; example?: unknown };

export type ResponseObject = {
  description?: string;
  content?: Record<string, MediaType>;
};

export type RequestBody = {
  description?: string;
  required?: boolean;
  content: Record<string, MediaType>;
};

export type SecurityRequirement = Record<string, string[]>;

export type Operation = {
  tags?: string[];
  summary?: string;
  description?: string;
  operationId?: string;
  deprecated?: boolean;
  parameters?: Parameter[];
  requestBody?: RequestBody;
  responses?: Record<string, ResponseObject>;
  security?: SecurityRequirement[];
};

export type SecurityScheme = {
  type: string;
  in?: string;
  name?: string;
  scheme?: string;
  description?: string;
};

export type OpenApiDocument = {
  openapi: string;
  info: { title: string; version: string; description?: string };
  servers?: { url: string; description?: string }[];
  tags?: { name: string; description?: string }[];
  paths: Record<string, Record<string, Operation>>;
  components?: {
    schemas?: Record<string, Schema>;
    securitySchemes?: Record<string, SecurityScheme>;
  };
  security?: SecurityRequirement[];
};

export const METHODS = [
  'get',
  'post',
  'put',
  'patch',
  'delete',
  'head',
  'options',
  'trace',
] as const;
export type Method = (typeof METHODS)[number];

const isMethod = (key: string): key is Method =>
  (METHODS as readonly string[]).includes(key);

/** `#/components/schemas/PriceResponse` → `PriceResponse`. */
export const refName = (ref: string): string =>
  ref.slice(ref.lastIndexOf('/') + 1);

/** The schema a `$ref` points at, or `undefined` when the document lacks it. */
export function resolveRef(
  doc: OpenApiDocument,
  ref: string,
): Schema | undefined {
  return doc.components?.schemas?.[refName(ref)];
}

/**
 * Follow `$ref`s until a schema with a body. Returns the schema and the name
 * of the component it came from, if any — the name is what the reference
 * links to. Bounded, so a document with a reference cycle cannot hang the
 * page.
 */
export function deref(
  doc: OpenApiDocument,
  schema: Schema | undefined,
): { schema: Schema | undefined; name?: string } {
  let current = schema;
  let name: string | undefined;
  for (let hops = 0; current?.$ref && hops < 8; hops++) {
    name = refName(current.$ref);
    current = resolveRef(doc, current.$ref);
  }
  return { schema: current, name };
}

/**
 * `type` as OpenAPI 3.1 writes it — a string or a list that may include
 * `"null"` — split into the non-null type and whether null is allowed.
 */
function splitType(schema: Schema): { type?: string; nullable: boolean } {
  const types = Array.isArray(schema.type)
    ? schema.type
    : schema.type
      ? [schema.type]
      : [];
  const nullable = types.includes('null');
  const type = types.find((t) => t !== 'null');
  return { type, nullable };
}

/**
 * The one option in a `oneOf`/`anyOf` that is not `null`, if that is all the
 * union is — `utoipa`'s spelling of an optional reference.
 */
function nonNullVariant(schema: Schema): {
  variant: Schema | undefined;
  nullable: boolean;
} {
  const union = schema.oneOf ?? schema.anyOf;
  if (!union) return { variant: undefined, nullable: false };
  const rest = union.filter((s) => !isNullSchema(s));
  return {
    variant: rest.length === 1 ? rest[0] : undefined,
    nullable: rest.length < union.length,
  };
}

/** `{ type: "null" }` and nothing else — the `null` half of an optional. */
function isNullSchema(schema: Schema): boolean {
  if (schema.$ref || schema.properties || schema.enum || schema.items) {
    return false;
  }
  const types = Array.isArray(schema.type)
    ? schema.type
    : schema.type
      ? [schema.type]
      : [];
  return types.length === 1 && types[0] === 'null';
}

/**
 * A type label a reader can scan: `string`, `integer (int32)`,
 * `AssetListItem[]`, `PriceResponse`, `string | null`. Enum schemas that are
 * components keep their name (the reference links to their values); an
 * inline enum is its base type.
 */
export function typeLabel(
  doc: OpenApiDocument,
  schema: Schema | undefined,
): string {
  if (!schema) return 'any';
  if (schema.$ref) return refName(schema.$ref);
  const { variant, nullable: unionNullable } = nonNullVariant(schema);
  if (variant) {
    return `${typeLabel(doc, variant)}${unionNullable ? ' | null' : ''}`;
  }
  if (schema.oneOf || schema.anyOf) {
    const parts = (schema.oneOf ?? schema.anyOf ?? []).map((s) =>
      typeLabel(doc, s),
    );
    return parts.join(' | ');
  }
  if (schema.allOf) {
    return schema.allOf.map((s) => typeLabel(doc, s)).join(' & ');
  }
  const { type, nullable } = splitType(schema);
  const suffix = nullable ? ' | null' : '';
  if (!type) return `any${suffix}`;
  if (type === 'array') return `${typeLabel(doc, schema.items)}[]${suffix}`;
  if ((type === 'integer' || type === 'number') && schema.format) {
    return `${type} (${schema.format})${suffix}`;
  }
  if (type === 'string' && schema.format)
    return `string (${schema.format})${suffix}`;
  return `${type}${suffix}`;
}

/**
 * The description to show for a property or parameter: its own, or — for
 * `oneOf: [null, $ref]`, where `utoipa` attaches the field's doc comment to
 * the reference variant rather than to the property — the variant's, and
 * failing both, the referenced component's.
 */
export function describe(
  doc: OpenApiDocument,
  schema: Schema | undefined,
): string | undefined {
  if (!schema) return undefined;
  if (schema.description) return schema.description;
  const { variant } = nonNullVariant(schema);
  if (variant?.description) return variant.description;
  return deref(doc, variant ?? schema).schema?.description;
}

/** The component a label should link to, if the schema is or wraps one. */
export function linkedComponent(
  schema: Schema | undefined,
): string | undefined {
  if (!schema) return undefined;
  if (schema.$ref) return refName(schema.$ref);
  const { variant } = nonNullVariant(schema);
  if (variant?.$ref) return refName(variant.$ref);
  if (schema.items?.$ref) return refName(schema.items.$ref);
  return undefined;
}

/**
 * An example value for a schema. `example` wins where the document gives
 * one; otherwise the first enum member, a placeholder per primitive type,
 * and objects and arrays built from their parts. Depth-bounded so a
 * recursive schema produces a finite document.
 */
export function exampleOf(
  doc: OpenApiDocument,
  schema: Schema | undefined,
  depth = 0,
  hops = 0,
): unknown {
  if (!schema || depth > 8 || hops > 16) return null;
  if (schema.example !== undefined) return schema.example;
  // Following a reference or unwrapping an optional is not nesting — only
  // stepping into a property or an item is, or a document three references
  // deep would bottom out at the first object with properties. `hops` bounds
  // those sideways steps on their own: a component that refers to itself
  // through `allOf`/`oneOf` rather than through a property would otherwise
  // recurse until the stack gave out and the page went blank.
  if (schema.$ref) {
    return exampleOf(doc, resolveRef(doc, schema.$ref), depth, hops + 1);
  }
  if (schema.default !== undefined) return schema.default;
  if (schema.enum?.length) return schema.enum[0];
  const { variant } = nonNullVariant(schema);
  if (variant) return exampleOf(doc, variant, depth, hops + 1);
  if (schema.oneOf?.length) {
    return exampleOf(doc, schema.oneOf[0], depth, hops + 1);
  }
  if (schema.anyOf?.length) {
    return exampleOf(doc, schema.anyOf[0], depth, hops + 1);
  }
  if (schema.allOf?.length) {
    return Object.assign(
      {},
      ...schema.allOf.map((s) => {
        const value = exampleOf(doc, s, depth, hops + 1);
        return value && typeof value === 'object' ? value : {};
      }),
    );
  }
  const { type } = splitType(schema);
  switch (type) {
    case 'object': {
      const out: Record<string, unknown> = {};
      for (const [name, prop] of Object.entries(schema.properties ?? {})) {
        out[name] = exampleOf(doc, prop, depth + 1, hops);
      }
      return out;
    }
    case 'array':
      return [exampleOf(doc, schema.items, depth + 1, hops)];
    case 'string':
      return schema.format === 'date-time' ? '2026-01-01T00:00:00Z' : 'string';
    case 'integer':
      return schema.minimum ?? 0;
    case 'number':
      return schema.minimum ?? 0;
    case 'boolean':
      return true;
    default:
      // A schema with properties and no `type` is an object in 3.1 too.
      if (schema.properties) {
        return exampleOf(doc, { ...schema, type: 'object' }, depth, hops + 1);
      }
      return null;
  }
}

/** Whether a request to this operation must carry a key. */
export function requiresKey(doc: OpenApiDocument, op: Operation): boolean {
  const requirements = op.security ?? doc.security ?? [];
  // `[{}]` is OpenAPI's spelling of "anonymous allowed" — an empty
  // requirement is one that needs nothing.
  return (
    requirements.length > 0 &&
    requirements.every((r) => Object.keys(r).length > 0)
  );
}

export type OperationEntry = {
  id: string;
  method: Method;
  path: string;
  operation: Operation;
};

/** A DOM id for an operation — the TOC target and the row's anchor. */
export const operationId = (method: string, path: string): string =>
  `op-${method}-${path.replace(/[^a-z0-9]+/gi, '-').replace(/^-|-$/g, '')}`;

/** A DOM id for a tag section. */
export const tagId = (tag: string): string =>
  `tag-${tag.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}`;

/** A DOM id for a schema row. */
export const schemaId = (name: string): string =>
  `schema-${name.replace(/[^a-z0-9]+/gi, '-')}`;

/** Every operation in document order, methods in `METHODS` order per path. */
export function operations(doc: OpenApiDocument): OperationEntry[] {
  const out: OperationEntry[] = [];
  for (const [path, item] of Object.entries(doc.paths ?? {})) {
    for (const method of METHODS) {
      const operation = item[method];
      if (operation && isMethod(method)) {
        out.push({ id: operationId(method, path), method, path, operation });
      }
    }
  }
  return out;
}

export type TagGroup = {
  name: string;
  description?: string;
  operations: OperationEntry[];
};

/**
 * Operations grouped by their first tag, in the order the document lists its
 * tags; tags the document uses but never declares follow in order of first
 * use, and untagged operations close the list under `other`. The order is
 * the document's on purpose — `openapi/mod.rs` writes `ops` first because a
 * reader checking liveness should not scroll past prices to find it.
 */
export function groupByTag(doc: OpenApiDocument): TagGroup[] {
  const declared = doc.tags ?? [];
  const groups = new Map<string, TagGroup>();
  for (const tag of declared) {
    groups.set(tag.name, {
      name: tag.name,
      description: tag.description,
      operations: [],
    });
  }
  for (const entry of operations(doc)) {
    const name = entry.operation.tags?.[0] ?? 'other';
    let group = groups.get(name);
    if (!group) {
      group = { name, operations: [] };
      groups.set(name, group);
    }
    group.operations.push(entry);
  }
  return [...groups.values()].filter((g) => g.operations.length > 0);
}

/**
 * A summary without the method and path the row already shows. The
 * handler's summaries open with the route in backticks (``` `GET /assets` —
 * paginated… ```), which is right in a document read on its own and a
 * repetition next to a badge and a path; the dash goes with it.
 */
export function displaySummary(
  summary: string | undefined,
): string | undefined {
  if (!summary) return summary;
  const stripped = summary.replace(
    /^`(?:GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|TRACE)\s+[^`]*`\s*(?:—|-|–|:)?\s*/i,
    '',
  );
  if (!stripped) return summary;
  // The remainder was authored as the tail of a sentence that began with the
  // route ("`GET /health` — liveness probe."), so on its own it starts
  // lower-case and reads as a fragment. Only the first letter moves; a
  // remainder opening with code or a proper noun is unaffected.
  return stripped[0].toUpperCase() + stripped.slice(1);
}

/**
 * Strip the project's own bookkeeping out of a description before a reader
 * sees it: task numbers, ADR numbers, PR numbers, section marks into the
 * internal overview (`§4.2`, `general-overview §3.3 / §4.2`).
 *
 * **Belt and braces — nothing in the document carries these today.** The
 * published text comes from `openapi/descriptions.rs` and from
 * `#[utoipa::path]` attributes, written for an integrator, and
 * `every_published_text_is_present_and_reader_facing` in `tests/openapi.rs`
 * fails the build if one of these shapes reaches the document. This runs
 * anyway, because the handler's doc comments DO carry them for the next
 * maintainer and a regression that let one through would be invisible on
 * this side of the wire.
 *
 * A parenthetical that held nothing else goes with its parentheses; a bare
 * mention goes alone, and the spacing around it is tidied.
 */
export function scrubInternal(text: string): string {
  const marker =
    /(?:\btasks?\s*#?\d{3,4}\b|\bADRs?\s*#?\d{3,4}\b|\bPRs?\s*#\d+\b|§\s?\d+(?:\.\d+)*|\bgeneral-overview\b|\boverview\s+§|\b0\d{3}\b)/;
  return (
    text
      // Parentheticals with a marker inside: `(task 0135)`, `(overview §4.2)`,
      // `(general-overview §3.3 / §4.2)`, `(task 0119's exact-token policy)`.
      .replace(/\s*\(([^()]*)\)/g, (whole, inner: string) =>
        marker.test(inner) ? '' : whole,
      )
      // Bare mentions: `task 0135's`, `the 0135 carry bound`, `§5.5`,
      // `general-overview §3.3 / §4.2`, `ADR 0008`.
      .replace(
        /\b(?:general-overview|overview)\s*§\s?\d+(?:\.\d+)*(?:\s*\/\s*§\s?\d+(?:\.\d+)*)*/g,
        '',
      )
      .replace(/§\s?\d+(?:\.\d+)*(?:\s*\/\s*§\s?\d+(?:\.\d+)*)*/g, '')
      .replace(/\b(?:tasks?|ADRs?)\s*#?\d{3,4}(?:'s)?/g, '')
      .replace(/\bPRs?\s*#\d+/g, '')
      // A bare task id — `the 0135 carry bound` — but ONLY where the word in
      // front of it makes it one. `\b0\d{3}\b` on its own also matched the
      // tail of a decimal (`0.0500` → `0.`) and every other zero-padded
      // number, which would have quietly mangled a published description.
      .replace(/\b(the|in|from|see|per|task|tasks)\s+0\d{3}(?:'s)?\b/gi, '$1')
      // Tidy what the removals left behind.
      .replace(/\(\s*\)/g, '')
      .replace(/[ \t]+([,.;:)])/g, '$1')
      .replace(/([ \t])[ \t]+/g, '$1')
      .replace(/\bthe\s+(?=[,.;:])/g, '')
      .trim()
  );
}

/** `2xx` reads as success, `4xx` as the caller's problem, `5xx` as ours. */
export function statusTone(
  status: string,
): 'success' | 'warn' | 'error' | 'muted' {
  if (/^2/.test(status)) return 'success';
  if (/^4/.test(status)) return 'warn';
  if (/^5/.test(status)) return 'error';
  return 'muted';
}

/** The JSON media type of a body, or whichever comes first. */
export function primaryMedia(
  content: Record<string, MediaType> | undefined,
): { mediaType: string; media: MediaType } | undefined {
  if (!content) return undefined;
  const entries = Object.entries(content);
  const json = entries.find(([type]) => /json/i.test(type));
  const [mediaType, media] = json ?? entries[0] ?? [];
  return mediaType && media ? { mediaType, media } : undefined;
}

/** The header scheme partners authenticate with, if the document names one. */
export function apiKeyHeader(doc: OpenApiDocument): string | undefined {
  for (const scheme of Object.values(doc.components?.securitySchemes ?? {})) {
    if (scheme.type === 'apiKey' && scheme.in === 'header' && scheme.name) {
      return scheme.name;
    }
  }
  return undefined;
}
