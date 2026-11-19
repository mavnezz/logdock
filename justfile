set shell := ["bash", "-euo", "pipefail", "-c"]

# List the recipes
default:
    @just --list

# Format the backend code
[working-directory: 'backend']
fmt:
    cargo fmt --all

# Check formatting and lints; fails on any warning
[working-directory: 'backend']
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Run all backend tests
[working-directory: 'backend']
test:
    cargo test --workspace --locked

# Everything CI checks; the tests run measured, against the coverage thresholds
check: lint deny coverage-backend api-check web-check

# Licenses, advisories and sources of the crates (backend/deny.toml)
[working-directory: 'backend']
deny:
    cargo deny --locked check

# Both test suites, measured
coverage: coverage-backend coverage-web

# The backend's tests, measured; thresholds in scripts/coverage-check.sh.
# main.rs only wires the binary's commands; the feature tests run the rest.
[working-directory: 'backend']
coverage-backend:
    mkdir -p ../dev/coverage
    cargo llvm-cov --workspace --locked --json --summary-only \
        --ignore-filename-regex 'logdockd/src/main\.rs' \
        --output-path ../dev/coverage/backend.json
    ../scripts/coverage-check.sh ../dev/coverage/backend.json

# A browsable report of the backend's coverage: dev/coverage/backend/html/index.html
[working-directory: 'backend']
coverage-html:
    cargo llvm-cov --workspace --locked --html --output-dir ../dev/coverage/backend

# The frontend's tests, measured; thresholds in frontend/vite.config.ts
[working-directory: 'frontend']
coverage-web:
    npm run -s test:coverage

# Run the server for development; its data lives in dev/data, the
# interface comes from `just web-dev` on http://localhost:5173
[working-directory: 'backend']
dev:
    LOGDOCK_DATA_DIR=../dev/data LOGDOCK_WEB_DIR= LOGDOCK_LISTEN=127.0.0.1:8080 \
    LOGDOCK_PUBLIC_URL=http://localhost:5173 LOGDOCK_LOG_FORMAT=text LOGDOCK_MIN_FREE_DISK=100MB \
        cargo run -p logdockd

# Print the OpenAPI document
[working-directory: 'backend']
openapi:
    @cargo run -q -p logdockd -- openapi

# Regenerate frontend/openapi.json and the TypeScript API types from the backend
api-types:
    cd backend && cargo run -q -p logdockd -- openapi > ../frontend/openapi.json
    cd frontend && npm run -s gen:api

# Fail if frontend/openapi.json or the TypeScript API types are stale
api-check:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    (cd backend && cargo run -q -p logdockd -- openapi) > "$tmp/openapi.json"
    diff -u frontend/openapi.json "$tmp/openapi.json"
    (cd frontend && npx openapi-typescript "$tmp/openapi.json" -o "$tmp/schema.d.ts" > /dev/null)
    diff -u frontend/src/shared/api/schema.d.ts "$tmp/schema.d.ts"

# Install the frontend dependencies exactly as locked
[working-directory: 'frontend']
web-install:
    npm ci

# Frontend dev server on http://localhost:5173; proxies /api to `just dev`
[working-directory: 'frontend']
web-dev:
    npm run dev

# Format the frontend code
[working-directory: 'frontend']
web-fmt:
    npm run -s format

# Frontend checks: formatting, lint, tests with their coverage, types and production build
[working-directory: 'frontend']
web-check:
    npm run -s format:check
    npm run -s lint
    npm run -s test:coverage
    npm run -s build

# Build the Docker image as logdock:local
image:
    docker build -f docker/Dockerfile -t logdock:local .

# Start Logdock with Docker Compose (compose.yaml), building the image
up:
    docker compose up -d --build

# End-to-end check with the Laravel test app in Docker (compose.dev.yaml);
# --keep leaves it running
e2e *args:
    node dev/e2e/run.mjs {{args}}

# Logdock, the Laravel test app and Mailpit in Docker, left running
dev-stack:
    node dev/e2e/run.mjs --keep
