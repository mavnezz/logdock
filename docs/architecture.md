# Logdock Architecture

> Status: draft of 2026-11-18.
> **[Assumption]** marks assumptions, **[Open]** questions still to be decided, **[Later]** features deliberately
> postponed, **[Verify]** statements not yet confirmed against documentation or source code.
> Everything else about Laravel, Monolog, the Sentry SDKs and protocol, Bugsink, Nightwatch, SQLite and the
> forwarders was checked against their current documentation and source code on 2026-11-18 (sources in 14).

Logdock is a self-hosted service that collects the logs, errors and performance data of Laravel applications in one
place. Every container of every application and stage sends to it, and one web interface searches across all of them.
It runs as a single Docker container that keeps its data in one directory.

---

## 1. Scope

### 1.1 Goals

| Area | What Logdock does | Replaces |
|---|---|---|
| Logs | stores the Laravel logs of all apps, containers and stages; full-text search, filters, live view, related entries by trace | `docker exec` and `storage/logs/laravel.log` |
| Errors | groups exceptions into issues, tracks their state across releases, notifies on new and returning ones | Bugsink |
| Monitoring | requests, jobs, queries, outgoing requests, cache and scheduled tasks: throughput, durations, failures | the Nightwatch-like part (R2) |

All three arrive through the Sentry SDK the apps already use (3.1). The trace ID that the SDK gives every request, job
and command links them (4.4).

### 1.2 Non-goals

- Session replay, profiling, and distributed tracing as a subsystem of its own.
- An SDK or Composer package of our own.
- High availability or clustering. Logdock is one container with one data directory.
- Collecting every Docker, Nginx or system log automatically. Laravel's own logs come first. Other sources can be
  added later through a forwarder (3.6), and none of them is required.

### 1.3 Releases and Rollout

Logdock is developed and tested locally, against a Laravel test app in Docker Compose (11). Once a release is
complete, it is tried on the stages of the server environment. It then replaces Bugsink in one step, when the apps'
DSN moves from Bugsink to Logdock. Logs and errors share that DSN, so there is no phase in which logs go to Logdock
while errors still go to Bugsink. The first release must therefore cover errors as well as logs.

| Release | Content |
|---|---|
| **R1** | logs and errors: everything needed to switch Bugsink off and stop reading log files inside containers |
| **R2** [Later] | monitoring: requests, jobs, queries, outgoing requests, cache, scheduled tasks |
| **R3** [Later] | forwarders for Docker's stdout/stderr and Nginx, a buffered log mode, alert rules beyond issues |

---

## 2. Target Architecture

### 2.1 Runtime View

```text
Laravel containers (web · horizon · scheduler), per app and stage
  sentry-laravel: event · log · transaction (R2) · check_in (R2)          forwarders (R3, optional)
        │ HTTPS  POST /api/<project-id>/envelope/                          │ HTTPS  POST /api/v1/ingest/logs
        v                                                                  v
reverse proxy (TLS, existing) ──────────────► logdockd   one container, one port, never root
                                               ├─ ingest: key → project and stage, limits, parsing, redaction
                                               ├─ bounded queue → one writer task, batched transactions
                                               ├─ SQLite  /data/logdock.db               accounts, projects, keys, issues, rollups
                                               │          /data/segments/<kind>/<day>.db  logs, events (R2: spans), FTS5
                                               ├─ background: retention, storage limit, rollups, notifications
                                               └─ web interface (Vue), /api/v1 (session), live view (SSE), /health/*
```

### 2.2 Endpoints

| Path | Used by | Authentication |
|---|---|---|
| `POST /api/<project-id>/envelope/` | Sentry SDKs | DSN key (`X-Sentry-Auth`, `sentry_key`) |
| `POST /api/v1/ingest/logs` | forwarders (R3), scripts | `Authorization: Bearer <ingest key>` |
| `/api/v1/…` | web interface | session cookie and CSRF token |
| `GET /api/v1/logs/stream` | live view | session; Server-Sent Events |
| `GET /health/live`, `GET /health/ready` | Docker, monitoring | none; the answers hold no data |
| `/` | web interface | static files served by `logdockd` |

The deprecated `/api/<project-id>/store/` endpoint is not offered, because sentry-php sends envelopes only.

### 2.3 Container Image

- Multi-stage build: Rust and Node in the build stages. The runtime image holds only the binary and the built
  frontend, on a slim Debian 13 base. [Open] Distroless `cc` or `debian:trixie-slim`, decided in step 0.
- The image runs as an unprivileged user. `compose.yaml` sets `user: "${PUID}:${PGID}"`, so a bind-mounted data
  directory belongs to the host's user and nothing in the container runs as root.
- `HEALTHCHECK` runs `logdockd health`, which asks `/health/live`. The image needs no curl.
- On SIGTERM `logdockd` stops accepting requests, drains the queue within `LOGDOCK_SHUTDOWN_TIMEOUT`, commits,
  checkpoints the WAL and exits. `stop_grace_period` in `compose.yaml` is longer than that timeout, and the container
  has `restart: unless-stopped`.
- Its own logs go to stdout as JSON lines (`tracing-subscriber`).
- There is no TLS in the container; the existing reverse proxy terminates it. The live view uses Server-Sent Events,
  so the proxy must not buffer `/api/v1/logs/stream`. Logdock sends `X-Accel-Buffering: no` for Nginx.

### 2.4 Configuration

Configuration comes from environment variables only, documented in `.env.example`. These defaults are initial values
and are checked against the benchmark in 5.6.

| Variable | Default | Purpose |
|---|---|---|
| `LOGDOCK_PORT` | `8080` | host port in `compose.yaml` |
| `LOGDOCK_PUBLIC_URL` | – | the address behind the proxy, for the DSNs shown in the interface and for secure cookies |
| `LOGDOCK_TRUSTED_PROXIES` | empty | addresses (CIDR) whose `X-Forwarded-For` is believed |
| `LOGDOCK_RETENTION_LOGS_DAYS` | `30` | log entries |
| `LOGDOCK_RETENTION_EVENTS_DAYS` | `90` | error events; issues themselves stay |
| `LOGDOCK_MAX_STORAGE` | `50GiB` | above this, segments are deleted, oldest first |
| `LOGDOCK_MIN_FREE_DISK` | `2GiB` | below this, ingest answers `503` and readiness fails |
| `LOGDOCK_INGEST_MAX_BODY` | `1MiB` | request body as sent (compressed) |
| `LOGDOCK_INGEST_MAX_DECOMPRESSED` | `20MiB` | request body after decompression |
| `LOGDOCK_INGEST_QUEUE` | `100000` | items waiting for the writer |
| `LOGDOCK_SHUTDOWN_TIMEOUT` | `20s` | time to drain the queue on SIGTERM |
| `LOGDOCK_SESSION_IDLE`, `LOGDOCK_SESSION_LIFETIME` | `12h`, `7d` | when sessions end |
| `LOGDOCK_SETUP_CODE` | random | the first setup's code, for automated setups |
| `LOGDOCK_SMTP_URL` | – | notifications by e-mail |
| `PUID`, `PGID` | `1000` | owner of the data directory |

### 2.5 Data Directory

```text
/data/
├── logdock.db              accounts, sessions, projects, keys, issues, rollups, segment catalog
└── segments/
    ├── logs/2026-11-18.db
    ├── events/2026-11-18.db
    └── traces/…            (R2)
```

---

## 3. Ingest

### 3.1 Why the Sentry Protocol

sentry-laravel already sends almost everything Logdock shows, over a protocol documented on develop.sentry.dev:

| Logdock shows | sentry-laravel sends | Release |
|---|---|---|
| errors | `event` items | R1 |
| logs | `log` items from the `sentry_logs` log channel (since 4.15). Monolog's `context` and `extra` become attributes, and every entry carries a `trace_id` | R1 |
| requests, jobs, SQL queries, outgoing HTTP requests, cache, notifications, views | `transaction` items with spans, once `traces_sample_rate` is set. SQL bindings are off by default | R2 |
| scheduled tasks | `check_in` items from `->sentryMonitor()`, or for every task with `crons.monitor_scheduled_tasks` (since 4.30), including the task's crontab | R2 |
| mail, Artisan commands, users | partly: commands as breadcrumbs, users only with `send_default_pii` | R2 [Verify] against real payloads |

| Option | Pros | Cons |
|---|---|---|
| **Sentry ingest protocol** (chosen) | the apps already have sentry-laravel; one SDK sends errors, logs, transactions and check-ins; the protocol is documented; correlation by trace ID is built in | we implement a subset of a protocol we do not own, so SDK updates must be tested (11) |
| Nightwatch's package and agent | MIT-licensed; it records exactly what Nightwatch shows | the ingest protocol is undocumented; "Nightwatch is a fully-managed product"; a token is required; the backend address is only an undocumented variable |
| An SDK of our own | full control | a non-goal (1.2): a lot of PHP to write and maintain |
| Monolog's transports | no code on the Laravel side | none speaks HTTPS with a key per app. SyslogUdp is UDP without authentication or TLS. Socket is raw TCP whose TLS options cannot be set. Gelf needs an extra package. Monolog 3.13 has no generic HTTP handler |
| stdout/stderr plus a forwarder | the twelve-factor way | under PHP-FPM, workers' output is discarded by default (`catch_workers_output = no`), and lines longer than `log_limit` (1024 bytes) are split; a forwarder is needed on every host |

### 3.2 Envelope Endpoint

- **Authentication:** `X-Sentry-Auth: Sentry sentry_key=<key>, …`, which is what sentry-php sends. The query string
  form and the envelope header's `dsn` are accepted too. A missing, unknown or revoked key gets `403`.
- **Compression:** gzip (sentry-php's default), deflate, br and zstd. The decompressed size is capped.
- **Parsing:** a header line, then items with a `type` and an optional `length`. An item without a length runs to the
  next newline. Unknown item types are skipped and counted, never treated as an error.
- **Items:**

  | Item | R1 | R2 |
  |---|---|---|
  | `log` | stored as log entries. One item may hold up to 1000 entries: the protocol allows at most 100, but sentry-php 4.x sends its whole buffer in one item | |
  | `event` | stored and grouped into issues | |
  | `transaction` | counted and dropped | stored and rolled up |
  | `check_in` | counted and dropped | monitors |
  | anything else (`attachment`, `client_report`, `profile`, `span`, `trace_metric`, …) | counted and dropped | `client_report` shown as the SDK's own drops [Later] |

- **Answer:** `200` with `{"id": "<event id>"}`. The SDK only looks at the status code.
- **When it answers:** once the items are in the queue, not after the commit. The SDK never retries, and a quick
  answer frees the PHP worker sooner. If Logdock crashes, what was still in the queue is lost.
- **Overload:** `429` with `Retry-After` and `X-Sentry-Rate-Limits`. sentry-php then pauses those categories for that
  PHP process (60 seconds if neither header is set) and drops what it would have sent in the meantime.

### 3.3 NDJSON Endpoint

One JSON object per line:

```json
{"timestamp":"2026-11-18T09:15:02.123456+00:00","level":"warning","message":"Payment declined","service":"horizon","host":"docker-host-02","container":"orders-horizon-1","release":"4f2c1e9","trace_id":"5b8efff798038103d269b633813fc60c","attributes":{"order_id":42}}
```

Lines in the form of Monolog's `JsonFormatter` (`message`, `context`, `level`, `level_name`, `channel`, `datetime`,
`extra`) are accepted as well, so a JSON log file can be shipped unchanged (R3). This endpoint answers after the
commit. Forwarders retry until they get a 2xx answer, which gives at-least-once delivery (3.6).

### 3.4 Keys and Scope

- A project is one application. Each project has environments (its stages). A key belongs to exactly one project and
  one environment.
- The key alone decides the project and the environment. An event whose `environment` differs from the key's is
  rejected and counted. The metadata a client sends is never used for authorization.
- `service`, `host`, the container (`server.address`, the container's hostname by default) and the release only
  describe where an entry came from.
- A key is 32 random bytes. It is shown once and stored as a SHA-256 hash; that is enough for a random key of this
  length, which needs no slow password hash. A scope may have several keys, so keys can be rotated. Keys can be revoked,
  and their last use is recorded.
- The DSN shown in the interface is `https://<key>@<public host>/<project-id>`.

### 3.5 Limits and Visibility

| Limit | Initial value [Assumption] |
|---|---|
| request body, as sent / decompressed | 1 MiB / 20 MiB |
| entries per `log` item | 1000 |
| message | 16 KiB; longer messages are cut and marked |
| attributes per entry / length of one value | 100 / 4 KiB |
| one error event, decompressed | 1 MiB |
| frames per stack trace | 250 |
| rate per key | token bucket, 1000 items/s, bursts of 5000 |
| queue | `LOGDOCK_INGEST_QUEUE` |

For each key Logdock counts what it accepted, and what it rejected by reason: authentication, environment, size, rate,
parsing or disk. It also keeps the time and reason of the last rejection. These counts are shown on the project's page
and on an ingest status page. `/health/ready` fails when the writer stalls or free disk falls below
`LOGDOCK_MIN_FREE_DISK`.

### 3.6 Delivery Guarantees

| Mode | Path | Guarantee |
|---|---|---|
| **Direct** (R1, the default) | SDK → Logdock | Best effort. The SDK sends once, without retrying. When Logdock is down or answers `429`, the data is lost. |
| **Buffered** (R3) | Monolog JSON file → Vector → NDJSON endpoint. Vector's `file` source keeps checkpoints in its `data_dir`; its disk buffer needs at least about 256 MB; its `http` sink retries on 408, 429 and 5xx (except 501) | At least once; duplicates are possible. Logs only, not errors or transactions. |

In direct mode the stack channel keeps writing the local `laravel.log` as well, but nothing reads it back. Whatever
Logdock missed exists only in that file.

---

## 4. Data Model

### 4.1 Log Entry

| Field | From a Sentry `log` item | From NDJSON |
|---|---|---|
| time (UTC, microseconds) | `timestamp` | `timestamp` or `datetime` |
| received at | the server's clock | the server's clock |
| project, environment | the key | the key |
| service (web, horizon, scheduler, …) | attribute `service` (8.1) | `service` |
| host | attribute `host` (8.1) | `host` |
| container | attribute `server.address` | `container` |
| release | attribute `sentry.release` | `release` |
| level | `level` | `level` or `level_name` |
| message | `body` | `message` |
| message template | attribute `sentry.message.template`, when there are placeholders | – |
| attributes | all other attributes, flattened: the log call's context, Laravel `Context`, … | `attributes`, or `context` and `extra` |
| trace ID | `trace_id` | `trace_id` |
| user | `user.id`, only when the app sends personal data | – |

When Laravel reports an exception it also logs it. The log entry keeps the message. The exception itself arrives as an
`event` and is found through the trace ID. [Verify] What exactly Sentry's logs handler keeps of a logged exception, and
how it maps Monolog's eight levels onto the protocol's six.

### 4.2 Errors and Issues (R1)

An event is stored redacted (6) and compressed. It holds:

- the SDK's event ID, the time, release, environment, service, host and container;
- the trace ID and the transaction (the route, or the job's class);
- the exception chain: type, value, and frames with filename, function, line, `in_app` and context line;
- the message, tags and contexts;
- the breadcrumbs. By default the SDK records logs, SQL queries (without bindings), cache, queue, HTTP client and
  notifications;
- the request: method, URL and allowed headers, but no body by default;
- the user, only when the app sends personal data.

**Grouping.** Logdock has its own, documented rules. They follow the order Sentry's documentation describes:
fingerprint, then stack trace, then exception, then message. Sentry does not specify its algorithm in full, so issues
will not match Sentry's or Bugsink's. Issues start fresh after the switch; Bugsink's history is not imported
[Assumption].

1. The SDK's `fingerprint`, when there is one (`{{ default }}` expands to rules 2–4).
2. The exception chain with in-app frames. For each exception its type, and for each in-app frame the normalized
   filename (the path below the project root) and the function. Line numbers are left out, so an unrelated edit does
   not split an issue.
3. Without in-app frames: the exception types and the first line of the value. Numbers, UUIDs and hex strings in the
   value are replaced.
4. Without an exception: the message template, otherwise the first line of the message with numbers replaced.

**Issue.** An issue has a title, a culprit (the first in-app frame), first and last seen, a count and the releases it
appeared in. Its latest event is kept in `logdock.db`, so the issue's page still works after its events have expired.
Its state:

- unresolved → resolved, either now or "in the next release". A new event after that marks it as regressed. For "in
  the next release", only an event from a later release counts.
- muted, until a date or for N more events. After that it is unresolved again.
- New and regressed issues are notified by e-mail, to a fixed list of addresses per project that an administrator
  keeps (also people without an account). E-mail is the only channel for now (decided on 2026-11-18).

### 4.3 Transactions, Spans and Check-ins (R2) [Later]

- Transactions are stored in segments. Each is a request or a job: name (route or job class), duration, status, trace
  ID, release, service and host.
- Spans are kept briefly (about 7 days). Rollups per minute are kept long: request count, errors and a histogram of
  durations, for each project, environment, kind and name. Percentiles come from these histograms. Sampled data is
  scaled up by its sample rate. [Verify] Where the payload states the rate.
- SQL spans are grouped by their normalized statement; outgoing requests are grouped by host.
- Check-ins become monitors per scheduled task, with the task's crontab. A run that fails, or does not happen when the
  schedule says it should, is reported.

### 4.4 Correlation

- **Trace ID.** sentry-php gives every log entry the trace ID of the current span. When tracing is off, it uses the
  trace ID of the scope's propagation context, so the ID exists without performance monitoring. Queued jobs continue
  the trace of the request that dispatched them: sentry-laravel puts `sentry_trace_parent_data` and
  `sentry_baggage_data` into the job's payload. A request, its logs, its error and the jobs it dispatched therefore
  share one trace ID, and "same trace" shows them together.
- **Own IDs.** Laravel's `Context` is added to every log record's `extra`, which becomes attributes. It also travels
  into queued jobs. A request ID or job ID put there can be searched like any other attribute. It is not suitable for
  per-container metadata (8.2).

---

## 5. Storage

### 5.1 SQLite in Day Segments

Logdock uses SQLite through sqlx with `sqlite-bundled`, as zendiloOS does. libsqlite3-sys 0.40 bundles SQLite 3.54.1
and compiles it with FTS5.

- **Segments.** Each data kind has one segment file per day of receipt. The writer writes only today's segments.
- **Catalog.** `logdock.db` keeps a catalog of the segments: kind, day, path, rows, size, the earliest and latest event
  time, and whether the segment is closed. A query picks the segments whose event times overlap the requested range,
  so entries that arrived late (a forwarder catching up) are found too.
- **Queries.** They run segment by segment, newest first, until a page is full. Counts and charts come from the rollups
  in `logdock.db`, not from the segments. No database is attached: SQLite allows 10 attached databases by default and
  125 at most.
- **Closing.** When a day ends, its segments are closed: FTS `optimize` and `PRAGMA optimize` run once, and no further
  writes happen. A closed segment never changes again.

### 5.2 Write Path

- A bounded queue feeds one writer task, which has its own connection. It writes a batch per transaction, up to 1000
  items or 200 ms [Assumption].
- The databases run in WAL mode with `synchronous=NORMAL`. A commit survives a crash of Logdock. After a power loss the
  last commits may be rolled back, but the database stays intact.
- Readers use a separate pool of read-only connections. In WAL mode they read while the writer writes. Readers that
  never let go would keep the WAL from being checkpointed, so reads are short and paged, and the writer checkpoints
  after its batches.

### 5.3 Full-text Search

FTS5 uses an external content table over the message, the exception text and the flattened attributes, so results can
show snippets with the matches highlighted. Structured filters use ordinary indexes, not FTS: project, environment,
service, host, level, time and trace ID.

Tokenizer and detail level, decided by the benchmark in step 3: `unicode61` with `tokenchars '_'` (so `order_id` is
one token) and full detail, which keeps phrase queries. An attribute term (`request_id:…`) also asks the index for its
value in the attributes column, then checks the key with `json_extract`. The options were:

| Option | Finds | Cost |
|---|---|---|
| `unicode61`, with `tokenchars` for identifiers such as `order_id` | words and prefixes (`Payment*`) | the smallest index |
| `trigram` | any substring of 3 or more characters; LIKE and GLOB can use the index | a larger index |
| `detail=column` or `detail=none` | no phrase or NEAR queries | a much smaller index: in SQLite's example 743 MiB with full detail, 340 MiB with column, 134 MiB with none |

### 5.4 Retention and Storage Limit

- **Retention.** A segment is deleted when its day falls out of retention. Deleting the file is instant and gives the
  space back. A `DELETE` would leave the pages on SQLite's freelist and would update FTS for every row.
- **Storage limit.** Above `LOGDOCK_MAX_STORAGE` the oldest segments go first, and the interface shows a notice.
- **Free disk.** Below `LOGDOCK_MIN_FREE_DISK` the ingest endpoints answer `503` and readiness fails.
- **Issues** stay after their events have expired (4.2).

### 5.5 Backup and Restore

- **Backup.** Logdock has no backup command of its own (decided on 2026-11-18). `docs/operations.md` describes how
  the host's tools save the data directory: closed segments are copied as files, because they do not change; the main
  database and today's segments with `sqlite3 … ".backup"` or `VACUUM INTO`, or with the container stopped. Copying an
  open database in WAL mode with `cp` is not safe.
- **Copy to another host.** `sqlite3_rsync` (since SQLite 3.47) can copy a live database there.
- **Scheduled backups** are not part of Logdock (decided on 2026-11-18). The host's backup tool saves the data
  directory as `docs/operations.md` describes.
- **Restore.** Stop the container, replace the data directory with the backup's contents, start.

### 5.6 Sizing [Assumption]

- **Volume.** The largest app writes 100,000 to 1 million log lines a day (an estimate given on 2026-11-18). That is an
  average of up to about 12 a second; peaks are assumed at 20 times that, about 250 a second.
- **Benchmark target (step 3).** Sustain 10 times that peak (2,500 entries a second for 10 minutes) on the target
  server without the queue growing. The results go into `docs/testing.md`. No throughput is claimed before then.
- **Disk.** Measured in step 3 (docs/testing.md): 545 bytes per entry including the index. At 1 million entries a
  day that is about 0.55 GB a day, and 16 GB for 30 days, for that app alone.
- **Measured throughput.** 7,800–9,700 entries a second on a development workstation, three times the target. The
  production server is still to be measured [Open].

### 5.7 Alternatives Considered

| Option | Pros | Cons |
|---|---|---|
| **Day segments in SQLite** (chosen) | retention and the storage limit are file deletions; closed segments are immutable, so backups are cheap; no second container | queries span files, so the code merges results |
| One SQLite database, `DELETE` for retention | simplest | the file does not shrink without VACUUM; deleting is expensive with FTS; every backup copies everything |
| Segments as attached databases | one SQL query across days | at most 10 attached databases by default, 125 at most |
| PostgreSQL, ClickHouse, Loki with Grafana, Parseable | built for more data or more writers | more containers and components; not justified by the volume in 5.6 |

The repositories sit behind ports (9). If spans in R2 turn out too large for SQLite, a different store for them would
change `logdock-infra` only.

---

## 6. Privacy and Redaction

Passwords, tokens, authorization headers and full request bodies must not be stored. Messages and exception texts can
contain them too. Logdock protects against this at three levels:

1. **In the app.** The SDK's defaults stay as they are: `send_default_pii=false`, and SQL bindings are off in
   breadcrumbs and spans.
2. **In Logdock, before anything is stored.** This level is the one that counts, because an app may be configured
   wrongly.
   - Keys are checked against a deny list, case-insensitively and as substrings, in attributes, context, extra, tags
     and headers: `password`, `passwd`, `secret`, `token`, `api_key`, `apikey`, `authorization`, `cookie`, `session`,
     `csrf`, `private_key`, `credit_card`, … Their values are replaced.
   - Free text (messages, exception values, breadcrumbs, URLs) is checked for patterns: `Bearer …`, JWTs (`eyJ…`),
     `user:password@` in URLs, query parameters with denied names, and card numbers that pass the Luhn check.
   - Request bodies are not stored by default. Headers are kept only from an allow list (`User-Agent`, `Content-Type`,
     `Accept`, `X-Request-Id`, …). Lengths are capped (3.5).
3. **Per project, optionally.** Only attributes on an allow list are kept.

Every replacement is marked (`[redacted:password]`) and counted per entry, so missing values are visible as such. The
rules are tested with fixtures. [Later] A page to test the rules against a pasted entry.

Patterns in free text catch only what they describe. They lower the risk; they do not rule it out.

---

## 7. Security and Access

- **Accounts.** Logdock has its own accounts:
  - passwords hashed with argon2;
  - a session cookie (`HttpOnly`, `Secure`, `SameSite=Strict`) and a CSRF token;
  - throttling of failed sign-ins;
  - [Later] two-factor sign-in with TOTP, as in zendiloOS.
- **First start.** `logdockd` writes a one-time setup code to its log (or takes `LOGDOCK_SETUP_CODE`). Whoever has
  it creates the first administrator at `/setup`.
- **Roles.** Administrators manage accounts, projects, keys and settings. Members read everything: all projects and
  all stages (decided for R1). [Later] Read rights per project and stage. The schema keeps reads behind one permission
  check, so this can be added without touching every query.
- **Ingest keys** are not accounts and can never sign in (3.4).
- **Behind the proxy.** `X-Forwarded-For` is believed only from `LOGDOCK_TRUSTED_PROXIES`. The audit log and the
  throttling use that address.
- **Audit log.** Sign-ins, and changes to accounts, keys, projects and settings.

---

## 8. Laravel Integration

This section is a sketch. It is tested against the test app in step 6.

### 8.1 Configuration

`.env`, the same for every container of an app and stage, except the last two lines:

```dotenv
SENTRY_LARAVEL_DSN=https://<key>@logdock.example.com/<project-id>
SENTRY_ENVIRONMENT=production
SENTRY_RELEASE=<release-or-commit>
SENTRY_SEND_DEFAULT_PII=false
# SENTRY_TRACES_SAMPLE_RATE stays unset until R2
LOG_STACK=daily,sentry_logs
SENTRY_LOG_LEVEL=info

# per container
LOG_SERVICE=horizon
LOG_SERVER=docker-host-02
```

`config/logging.php`:

```php
'service' => env('LOG_SERVICE', 'web'),
'host' => env('LOG_SERVER', 'unknown'),

'channels' => [
    'stack' => [
        'driver' => 'stack',
        'channels' => explode(',', (string) env('LOG_STACK', 'single')),
        'ignore_exceptions' => true,
    ],

    'sentry_logs' => [
        'driver' => 'sentry_logs',
        'level' => env('SENTRY_LOG_LEVEL', env('LOG_LEVEL', 'info')),
        'tap' => [App\Logging\ContainerMetadata::class],
    ],

    // daily, single, … as before
],
```

`app/Logging/ContainerMetadata.php`:

```php
<?php

declare(strict_types=1);

namespace App\Logging;

use Illuminate\Log\Logger;
use Monolog\LogRecord;

final class ContainerMetadata
{
    public function __invoke(Logger $logger): void
    {
        $metadata = ['service' => config('logging.service'), 'host' => config('logging.host')];

        $logger->pushProcessor(
            static fn (LogRecord $record): LogRecord => $record->with(extra: [...$record->extra, ...$metadata]),
        );
    }
}
```

In `AppServiceProvider::boot()`, the same metadata goes on errors and transactions as tags:

```php
\Sentry\configureScope(static function (\Sentry\State\Scope $scope): void {
    $scope->setTag('service', (string) config('logging.service'));
    $scope->setTag('host', (string) config('logging.host'));
});
```

In `config/sentry.php` the timeouts come down from their defaults of 2 s to connect and 5 s in total, for example to
`'http_connect_timeout' => 0.5` and `'http_timeout' => 2` [Assumption].

### 8.2 Things to Know

- **Per-container metadata comes from the container's environment**, through the `tap` above. Laravel's `Context` is
  not suitable for it: the dispatching process's context is restored inside a queued job, so a job's logs would say
  `web` instead of `horizon`.
- **`ignore_exceptions => true`.** Laravel's default is `false`. Without it, an exception thrown by a log handler
  reaches the code that logged, because Monolog rethrows it and Laravel sets no exception handler. Monolog's detection
  of logging loops is on by default.
- **Cost per request.** Under PHP-FPM, Symfony calls `fastcgi_finish_request()` before Laravel runs its terminate
  callbacks. The SDK therefore sends after the visitor has the response, but the worker is busy until the send is done
  or times out.
- **When logs are sent.** The SDK sends its buffered logs when a request, command or job ends.
  `SENTRY_LOG_FLUSH_THRESHOLD` sends them earlier, for long-running commands.
- **Horizon and the scheduler** use the app's normal logging. Horizon has no log settings of its own, and `horizon:work`
  is Laravel's queue worker. That worker does not reset Monolog's handlers between jobs; sentry-laravel flushes its own
  buffer after each job.
- **Versions.** Laravel 13, and Laravel 12 while it gets security fixes (until 2027-02-24); Monolog 3. sentry-laravel
  4.15 or later for logs, 4.30 or later for `crons.monitor_scheduled_tasks` (R2).
- **sentry-php 5.0.0-beta1** (2026-11-17) removes the hub API and `send_default_pii`. This section is revisited when
  5.0 is stable, and CI tests the SDK versions the apps use (11).

---

## 9. Backend Structure

The backend follows the rules of zendiloOS (`~/zendiloOS/docs/architecture.md`, section 2.1):

- domain-driven design in a hexagonal architecture, with crate boundaries enforcing the dependency rule;
- context crates without I/O;
- use cases as structs with an `execute` method, and no command or query bus;
- ports as `#[async_trait]` traits held as `Arc<dyn Port>`;
- no event bus: a context that must follow another declares an observer port;
- architecture tests over `cargo metadata` and the sources, parsed with `syn`.

These rules replace the TypeScript template with a command bus and integration events.

| Crate | Role | May depend on (internal) |
|---|---|---|
| `logdock-kernel` | shared kernel: IDs, time, `Secret`, the project and environment of a request, common port errors | – |
| `logdock-identity` | accounts, roles, sessions, passwords, throttling, audit | kernel |
| `logdock-projects` | projects, environments, ingest keys, quotas, redaction rules | kernel |
| `logdock-logs` | log entries: ingest rules, search, filters, correlation | kernel |
| `logdock-issues` | error events, grouping, issues and their states, notifications to send | kernel |
| `logdock-retention` | retention, the storage limit, segments, backups as use cases | kernel |
| `logdock-performance`, `logdock-monitors` (R2) | transactions, spans and rollups; check-ins and missed runs | kernel |
| `logdock-sentry` | the wire format only: envelope parser and payload types, like `zendiloos-agent-proto` | – |
| `logdock-infra` | the secondary adapters: sqlx repositories, segments, FTS5, migrations, mail, webhooks | kernel, contexts, sentry |
| `logdock-api` | the primary adapter: axum routes, DTOs, OpenAPI (utoipa), authentication, SSE | kernel, contexts, sentry |
| `logdockd` | composition root: configuration from the environment, signals, the `health` and `backup` commands | everything |
| `logdock-architecture` | tests only: checks this table and the rules above against the workspace | – |

A crate is created in the step that needs it; there are no empty placeholder crates. The toolchain follows zendiloOS:
a pinned `rust-toolchain.toml`, edition 2024, `rustfmt` with `max_width = 110`, `unsafe_code = "forbid"`, clippy with
`-D warnings`, cargo-deny, and sqlx's offline query check.

```text
backend/            Cargo workspace (crates/logdock-*, logdockd)
frontend/           Vue 3
dev/laravel/        Laravel test app for end-to-end tests and payload fixtures (step 2)
docker/Dockerfile
compose.yaml        the service as deployed
compose.dev.yaml    Logdock with the test app
docs/               architecture, testing, operations (backup and restore)
justfile            `just check` runs everything CI runs
```

---

## 10. Frontend

Vue 3 with `<script setup>`, TypeScript strict and Vite, like zendiloOS's frontend:

- Tailwind CSS 4 and shadcn-vue;
- Pinia for client state and `@tanstack/vue-query` for server state;
- API types from `openapi-typescript` and `openapi-fetch`, generated from utoipa;
- vue-i18n with English as the source language and German as the second one, chosen per account;
- lucide icons;
- Vitest;
- Prettier and ESLint, with `import-x/no-restricted-paths` so that features import only from `shared/`.

It uses plain routes; there is no desktop with windows.

| Page | Content |
|---|---|
| Logs | newest first, live view, full-text search, filters (project, stage, service, host, level, time range), detail with all attributes, "same trace" |
| Issues | list by state, last seen and count; detail with stack trace, events, tags, releases and the trace's logs |
| Projects | stages, keys, DSNs, ingest counts and rejections |
| Settings | accounts, retention and storage use, notifications, backups |
| Monitoring (R2) | requests, jobs, queries, outgoing requests, cache, scheduled tasks |

The error rate is a chart over the rollups on the logs and issues pages: entries per level and minute, and events per
issue and hour.

---

## 11. Testing

| Level | Purpose | How |
|---|---|---|
| Unit tests | domain rules: grouping, redaction, retention, limits | `cargo test` |
| Protocol tests | the envelope parser | fixtures of real payloads recorded from the test app's SDK versions; property and fuzz tests for broken input |
| API tests | endpoints, authentication, limits, answers | `logdockd` in-process with a temporary data directory |
| Architecture tests | crate boundaries and rules | `logdock-architecture` |
| Frontend tests | components and pages | Vitest with the API answered in the test |
| End-to-end | the real path from Laravel to the interface's API | `compose.dev.yaml`: the Laravel test app with sentry-laravel as web, horizon and scheduler; logs, an exception, and a job dispatched from a request (trace); Logdock stopped (the app keeps working, its added latency is measured) |
| Backup | backup and restore | backup, restore, compare counts |
| Benchmark | sizing in 5.6 | synthetic load and a copy of real logs; results in `docs/testing.md` |

`just check` runs formatting, lints, cargo-deny, the tests measured against coverage thresholds, the sqlx check, the
OpenAPI check, the frontend's checks and the Docker build. CI runs it on every push. The end-to-end tests run in CI
against each sentry-laravel version the apps use.

---

## 12. Delivery Plan

| Step | Result | Done when |
|---|---|---|
| 0. Foundation | workspace with kernel, api, infra, logdockd and architecture; `justfile` and CI; frontend skeleton; Dockerfile, `compose.yaml`, `.env.example`, `.dockerignore`; health endpoints; graceful shutdown | the image builds, `/health/*` answers, SIGTERM ends cleanly, `just check` passes |
| 1. Accounts and projects | first administrator by setup code; sign-in, sessions, CSRF; projects, stages, keys | API tests: revoked keys, the wrong stage rejected, members cannot manage |
| 2. Ingest core | `logdock-sentry` with fixtures from `dev/laravel`; envelope and NDJSON endpoints; limits; queue and writer; `429`; ingest counts | parser property and fuzz tests; a first load test |
| 3. Log storage | segments, FTS5, redaction, retention, storage limit; documented backup and restore with the host's tools | benchmark against 5.6 with results recorded; tokenizer decided |
| 4. Log interface | list, live view, filters, search, detail, "same trace", error rate | Vitest; tried with the test app's data |
| 5. Errors | events, grouping, issues and their states, regressions; issue pages with stack trace and logs; tag search; notifications | grouping tests with real exceptions; regression across releases |
| 6. Integration and R1 | section 8 tested end to end; `docs/operations.md`; trial on the stages | the test app and one real app on a stage work without Bugsink; **then Bugsink is replaced** |
| 7. Monitoring (R2) | transactions, spans, rollups, check-ins, their pages | its own plan |
| 8. Forwarders (R3) | NDJSON from Vector or Fluent Bit, buffered mode, Docker and Nginx logs | its own plan |

---

## 13. Decisions and Open Questions

Decided on 2026-11-18:

| # | Decision |
|---|---|
| D1 | Backend in Rust, following zendiloOS's guidelines; delivered as a Docker image |
| D2 | Data arrives through the Sentry ingest protocol; NDJSON is the second way in, for forwarders |
| D3 | SQLite in day segments, with FTS5 |
| D4 | Accounts of Logdock's own; every member reads all projects and stages; administrators manage |
| D5 | Developed and tested locally, then tried on the stages; Bugsink is replaced in one step, so R1 includes errors; no forwarding to Bugsink |
| D6 | Sizing from an estimate of 100,000 to 1 million log lines a day for the largest app |
| D7 | No backups inside Logdock; the host's tools save the data directory (docs/operations.md) |
| D8 | Notifications by e-mail only, to a fixed list of addresses per project |
| D9 | The interface in English and German |
| D10 | Every step of section 12 is built, monitoring (R2) and forwarders (R3) included, one commit per step |
| D11 | Issues belong to a project and one of its environments: each stage has its own issues and their states |
| D12 | FTS5 with `unicode61`, `tokenchars '_'` and full detail (step 3's benchmark) |

Open:

| # | Question | Needed by |
|---|---|---|
| O4 | Sample rate for monitoring; PHP-FPM or Octane | R2 |
| O5 | The product's name: "Logdock" now covers more than logs, and its availability has not been checked | before a public release |

---

## 14. Sources

Checked on 2026-11-18:

- Sentry protocol: <https://develop.sentry.dev/sdk/foundations/transport/authentication/>,
  <https://develop.sentry.dev/sdk/foundations/envelopes/>, <https://develop.sentry.dev/sdk/foundations/transport/>,
  <https://develop.sentry.dev/sdk/foundations/transport/rate-limiting/>, <https://develop.sentry.dev/sdk/telemetry/logs/>
- Sentry for Laravel: <https://docs.sentry.io/platforms/php/guides/laravel/logs/>,
  <https://docs.sentry.io/platforms/php/guides/laravel/configuration/options/>,
  <https://docs.sentry.io/concepts/data-management/event-grouping/>; sources of getsentry/sentry-laravel 4.30.0
  (`config/sentry.php`, `QueueIntegration.php`, `ConsoleSchedulingIntegration.php`, `Logs/LogsHandler.php`) and
  getsentry/sentry-php 4.37.1 (`Logs/LogsAggregator.php`, `Options.php`, `Transport/RateLimiter.php`)
- Laravel 13: <https://laravel.com/framework/docs/context>, <https://laravel.com/framework/docs/horizon>,
  <https://laravel.com/framework/docs/releases>; sources of `Illuminate/Log/LogManager.php`,
  `Log/Context/ContextLogProcessor.php`, `Queue/QueueServiceProvider.php`, `Foundation/Exceptions/Handler.php`
- Monolog 3.13: <https://github.com/Seldaek/monolog/tree/3.13.0/src/Monolog>
- PHP-FPM: <https://www.php.net/manual/en/install.fpm.configuration.php>
- Bugsink 2.6.3: <https://www.bugsink.com/docs/api/>; sources of `ingest/views.py`, `issues/urls.py`, `tags/models.py`
- Nightwatch: <https://nightwatch.laravel.com/docs/events>, <https://nightwatch.laravel.com/docs/guides/faqs>,
  <https://nightwatch.laravel.com/docs/environment-variables>
- SQLite: <https://sqlite.org/fts5.html>, <https://sqlite.org/wal.html>, <https://sqlite.org/pragma.html>,
  <https://sqlite.org/lang_vacuum.html>, <https://sqlite.org/rsync.html>, <https://sqlite.org/limits.html>; the
  libsqlite3-sys 0.40.0 `build.rs`
- Vector: <https://vector.dev/docs/reference/configuration/sources/file/>,
  <https://vector.dev/docs/reference/configuration/sinks/http/>, <https://vector.dev/docs/architecture/buffering-model/>
- Fluent Bit: <https://docs.fluentbit.io/manual/data-pipeline/outputs/http>,
  <https://docs.fluentbit.io/manual/data-pipeline/inputs/tail>
