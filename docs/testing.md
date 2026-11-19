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

## Benchmark

`cargo test -p logdock-infra --release -- --ignored --nocapture benchmark` writes entries
like a Laravel app's (a message, two attributes, a request ID) through the writer as the
SDK endpoint does, then searches them. `BENCH_ENTRIES` sets how many (default 200,000).

Measured on 2026-11-19 on a development workstation (Intel Pentium Gold 8505, 6 threads, SSD), with the
FTS5 tokenizer `unicode61` with `tokenchars '_'` and full detail:

| What | Result |
|---|---|
| Writing | 7,800–9,700 entries/s sustained, in batches of up to 2,000 per transaction |
| Disk | 545 bytes per entry, full-text index included |
| Full-text search (`deadlock`, 200,000 entries, first page) | 75–100 ms |
| Attribute search (`request_id:…`) | under 1 ms: the index narrows, `json_extract` checks |
| Newest page without filters | 1 ms |

Against the sizing of architecture.md, 5.6: the target of 2,500 entries/s is met three
times over; one million entries a day take about 0.55 GB a day, 16 GB for 30 days. The
server Logdock will run on was not measured yet [Open].

## End to end

Described with the Laravel test app (step 6 of the delivery plan).
