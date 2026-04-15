#!/usr/bin/env bash

set -e

API_URL="${API_URL:-http://localhost:3000}"
OPENAPI_ENDPOINT="/api/v1/openapi.json"

echo "Downloading OpenAPI spec from $API_URL$OPENAPI_ENDPOINT"

if ! curl -s "$API_URL$OPENAPI_ENDPOINT" -o /tmp/openapi.json; then
    echo "Error: Failed to download OpenAPI spec"
    exit 1
fi

echo "Generating TypeScript types..."

pnpm swagger-typescript-api \
    -p /tmp/openapi.json \
    -o src/api \
    -n types.ts \
    -t axios \
    --axios

echo "Cleaning up temporary file..."
rm -f /tmp/openapi.json

echo "Types generated successfully in src/api/types.ts"
