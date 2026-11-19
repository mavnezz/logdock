# Forwarders

Logs that Sentry's SDK does not send: what containers write to stdout and
stderr (Nginx, PHP-FPM, `queue:work`, …), and Laravel's own logs in
*buffered* mode. A forwarder sends them to Logdock's NDJSON endpoint
(`POST /api/v1/ingest/logs`, [architecture.md](architecture.md), 3.3). None of
this is required: the SDK alone covers Laravel's logs and errors
([laravel.md](laravel.md)).

Both configurations here are checked by `just e2e`
([testing.md](testing.md#end-to-end)) with Vector 0.62.0 and Fluent Bit 5.2.0:
Vector reads Nginx's output and Laravel's JSON log file, Fluent Bit gets the
queue worker's output through Docker's fluentd logging driver.

## One forwarder per app and stage

A forwarder sends with an ingest key, and the key decides the project and
stage of everything it sends. So a forwarder runs beside each app and stage,
in its compose project, with a key of its own (*Keys* tab, label it
`vector` or `fluent-bit`): the project's *Overview* then shows what each sent.

Logdock answers the NDJSON endpoint after the commit. Both forwarders retry
until they get a 2xx answer and keep what waits on disk (Vector: at least
256 MiB, Fluent Bit: up to 256 MiB here), so nothing is lost while Logdock is
away, but a line may arrive twice.

## What arrives

| Field | From |
|---|---|
| message | the line, or Monolog's `message`; for Nginx's access log `GET /path 200` |
| time | the container runtime's, or Monolog's `datetime` |
| level | Monolog's level; Nginx: 5xx `error`, 4xx `warn`, others `info`, its error log by its severity; otherwise stderr `error`, stdout `info` |
| service | the compose service (`com.docker.compose.service`), or Monolog's `extra.service` |
| container, host | the container's name; `LOGDOCK_HOST` |
| attributes | `stream`, `image`; for Nginx `status`, `method`, `path`, `client`, `size`, `referer`, `agent`; for Monolog its `context` and `extra` |

The project's redaction rules apply as to everything else; query strings
with `token=…` and the like are redacted.

**Twice the same line.** A container whose Laravel writes to stderr (the
`stderr` channel) and also sends through the SDK's `sentry_logs` channel
would arrive twice. Forward the output of containers that are not Laravel
(Nginx), or of Laravel containers whose stack has no `stderr` channel.

## Vector

[`forwarders/vector/vector.yaml`](../forwarders/vector/vector.yaml) and
[`compose.yaml`](../forwarders/vector/compose.yaml) beside it. Vector reads
the output of the containers of its compose project that carry the label
`logdock.forward: "true"`, through the Docker API (the socket is mounted
read-only):

```yaml
services:
  nginx:
    labels:
      logdock.forward: "true"
```

```bash
# .env of the app's stage
COMPOSE_PROJECT_NAME=orders-production
LOGDOCK_URL=https://logdock.example.com
LOGDOCK_FORWARDER_KEY=<key>
LOGDOCK_HOST=docker-host-02

docker compose -f compose.yaml -f forwarders/vector/compose.yaml up -d
```

Vector 0.62 reads `${…}` in its configuration only with
`VECTOR_DANGEROUSLY_ALLOW_ENV_VAR_INTERPOLATION=true`, which the compose file
sets: the configuration is this repository's and interpolates no event data.

## Fluent Bit

[`forwarders/fluent-bit/fluent-bit.yaml`](../forwarders/fluent-bit/fluent-bit.yaml)
and [`compose.yaml`](../forwarders/fluent-bit/compose.yaml) beside it. Fluent
Bit has no source for the Docker API; containers send to it with Docker's
fluentd logging driver instead, and `docker logs` keeps working:

```yaml
services:
  horizon:
    logging:
      driver: fluentd
      options:
        fluentd-address: 127.0.0.1:24224
        fluentd-async: "true"
        tag: docker.{{.Name}}
        labels: com.docker.compose.service
```

`fluentd-async` lets the container start while Fluent Bit is not up yet.
`LOGDOCK_HOST_NAME`, `LOGDOCK_PORT` (443) and `LOGDOCK_TLS` (`on`) name
Logdock. Fluent Bit leaves the level unset for stdout (Logdock reads `info`)
and sets `error` for stderr; Nginx's access log is parsed into fields but its
status does not set a level, as with Vector.

## Buffered mode

The SDK sends Laravel's log entries once and does not retry: when Logdock is
down, they are lost (architecture.md, 3.6). In buffered mode Laravel writes
Monolog's JSON lines to a file instead, and the forwarder ships them with its
retries and its disk buffer. Errors and transactions still go through the
SDK.

`config/logging.php`:

```php
'json' => [
    'driver' => 'daily',
    'path' => storage_path('logs/laravel.json'),
    'level' => env('LOG_LEVEL', 'info'),
    'days' => 3,
    'formatter' => Monolog\Formatter\JsonFormatter::class,
    'replace_placeholders' => true,
    'tap' => [App\Logging\ContainerMetadata::class],
],
```

```dotenv
LOG_STACK=json
```

`storage/logs` is the volume `laravel-logs`, which the forwarder mounts
read-only; both forwarders read `*.json` in it and remember how far they read.
Delete old files with `days`; the forwarders skip files older than two days.

## Checking it

- The project's *Overview* shows the forwarder's key with what it sent and
  refused.
- `docker compose logs vector` (or `fluent-bit`) shows refusals; `401` means
  a wrong or revoked key, `429` and `503` are retried.
