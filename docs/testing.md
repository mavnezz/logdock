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

`just e2e` (`dev/e2e/run.mjs`, Node 24 and Docker) runs Logdock and the Laravel
test app together, as `compose.dev.yaml` describes them:

- `logdock`, built from `docker/Dockerfile`, with Mailpit as its mail server;
- the test app (`dev/laravel`: Laravel 13, sentry-laravel, set up as
  [laravel.md](laravel.md) says) as `web`, `worker` (`queue:work`, named
  `horizon`) and `scheduler`, sharing a SQLite database;
- `capture` between the app and Logdock, which keeps what the SDK sent in
  `dev/captured/`.

The script starts from nothing, sets Logdock up, creates a project, a key and
an address to notify, starts the app with the key's DSN, calls its scenarios
(`/scenario/log`, `/fail`, `/handled`, `/job`, a scheduler run) and checks
through Logdock's API:

| Check | What it shows |
|---|---|
| a request's log entries arrive with service, host, release and stage | the tap and `.env` of laravel.md |
| a password in a log context arrives as `[redacted]` | redaction on ingest |
| entries below `SENTRY_LOG_LEVEL` stay out | the channel's level |
| an exception becomes an issue with its cause, tags and in-app frames | events and grouping |
| the request's log entries share the error's trace | "same trace" on the issue's page |
| a job's entries and errors say `horizon`, not `web` | per-container metadata in queued jobs |
| the scheduler's entries say `scheduler` | |
| a new issue is mailed | notifications through SMTP |
| ingest refused nothing, the envelopes were captured | |

`just e2e --keep` (or `just dev-stack`) leaves it all running: Logdock on
http://localhost:18480 (`dev@logdock.test`, `logdock-dev-password`), the app on
http://localhost:18400 and Mailpit on http://localhost:18425.

What the SDK sent is kept as fixtures in
`backend/crates/logdock-sentry/tests/fixtures/`; the parser's tests and the
feature test `takes_what_sentry_laravel_really_sends` replay them. To renew
them for a new SDK version, run `just e2e` and copy from `dev/captured/`.

The test app runs on the development machine only; the trial with a real app
on a stage is the last part of step 6 [Open].
