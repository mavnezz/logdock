# Testing

How Logdock is tested. Architecture and terms: [architecture.md](architecture.md).

## Levels

| Level | Purpose | How |
|---|---|---|
| Unit tests | domain rules: grouping, redaction, retention, limits, parsers | `cargo test`; part of `just check` |
| Protocol tests | the envelope parser | payloads recorded from the SDK versions the apps use, plus property tests for broken input |
| Feature tests | the assembled server through its router | `crates/logdockd/tests/`: `App::build` with a temporary data directory, requests through the real router |
| Architecture tests | crate boundaries and rules | `logdock-architecture`; part of `just check` |
| Frontend tests | components and pages | Vitest with the API answered in the test (`src/shared/testing`) |
| End-to-end | from Laravel to the interface's API | `compose.dev.yaml`, see [End to end](#end-to-end) |
| Image | the image starts, is healthy, stops cleanly | CI's *Docker image* job |

`just check` runs formatting, lints, cargo-deny, the backend's tests measured
against the coverage thresholds (`scripts/coverage-check.sh`), the check that
the frontend's API types match the backend, and the frontend's checks
(formatting, lint, tests with their thresholds, types, production build). CI
runs it on every push and builds and starts the image.

## End to end

Described with the Laravel test app (step 6 of the delivery plan).
