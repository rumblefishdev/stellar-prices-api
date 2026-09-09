import type { OpenApiDocument } from './openapi';

/**
 * A document in the handler's own dialect — the constructs `utoipa` emits
 * for this API, one of each. `packages/prices-api/tests/openapi.rs` pins the
 * served document; this pins that the page can read it.
 */
export const FIXTURE: OpenApiDocument = {
  openapi: '3.1.0',
  info: { title: 'Fixture API', version: '9.9.9', description: 'A fixture.' },
  servers: [{ url: 'https://api.example' }],
  tags: [
    { name: 'ops', description: 'Operational endpoints' },
    { name: 'assets', description: 'Asset metadata' },
  ],
  security: [{ api_key: [] }],
  paths: {
    '/health': {
      get: {
        tags: ['ops'],
        summary: '`GET /health` — liveness probe.',
        security: [{}],
        responses: { '200': { description: 'Alive' } },
      },
    },
    '/v1/assets': {
      get: {
        tags: ['assets'],
        summary: 'List assets.',
        description: 'Paginated. Use `cursor` for the next page.',
        parameters: [
          {
            name: 'type',
            in: 'query',
            description: 'classic | soroban | all (default all)',
            required: false,
            schema: { $ref: '#/components/schemas/TypeFilter' },
          },
          {
            name: 'limit',
            in: 'query',
            description: '1..=200 (default 50)',
            schema: {
              type: 'integer',
              format: 'int32',
              minimum: 1,
              maximum: 200,
            },
          },
        ],
        responses: {
          '200': {
            description: 'Asset list page',
            content: {
              'application/json': {
                schema: { $ref: '#/components/schemas/AssetListResponse' },
              },
            },
          },
          '400': {
            description: 'Invalid query parameter',
            content: {
              'application/json': {
                schema: { $ref: '#/components/schemas/ErrorEnvelope' },
              },
            },
          },
        },
      },
    },
    '/v1/prices/batch': {
      post: {
        tags: ['prices'],
        summary: 'Batch prices.',
        requestBody: {
          required: true,
          content: {
            'application/json': {
              schema: { $ref: '#/components/schemas/BatchRequest' },
            },
          },
        },
        responses: { '200': { description: 'Prices' } },
      },
    },
  },
  components: {
    securitySchemes: {
      api_key: { type: 'apiKey', in: 'header', name: 'x-api-key' },
    },
    schemas: {
      TypeFilter: {
        type: 'string',
        description: 'Which assets.',
        enum: ['classic', 'soroban', 'all'],
      },
      AssetListItem: {
        type: 'object',
        required: ['asset_code'],
        properties: {
          asset_code: {
            type: 'string',
            description:
              'The code. Two shapes:\n\n* `native` — the lumens\n  themselves.\n* `CODE:ISSUER` — anything else.',
            example: 'USDC',
          },
          price_usd: { type: ['string', 'null'], description: 'Latest price.' },
          stream: {
            oneOf: [{ type: 'null' }, { $ref: '#/components/schemas/Stream' }],
          },
          updated_at: { type: 'string', format: 'date-time' },
        },
      },
      Stream: {
        type: 'object',
        properties: {
          ledger: { type: 'integer', format: 'int64', minimum: 1 },
        },
      },
      AssetListResponse: {
        type: 'object',
        required: ['data', 'has_more'],
        properties: {
          data: {
            type: 'array',
            items: { $ref: '#/components/schemas/AssetListItem' },
          },
          cursor: { type: ['string', 'null'] },
          has_more: { type: 'boolean' },
        },
      },
      ErrorEnvelope: {
        type: 'object',
        required: ['code', 'message'],
        properties: {
          code: { type: 'string', example: 'invalid_id' },
          message: { type: 'string' },
          details: { description: 'Optional structured context.' },
        },
      },
      BatchRequest: {
        type: 'object',
        required: ['assets'],
        properties: {
          assets: {
            type: 'array',
            items: { type: 'string' },
            minItems: 1,
            maxItems: 100,
          },
        },
      },
    },
  },
};
