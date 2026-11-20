# Operations

How Logdock runs, is reached, saved and restored. Architecture:
[architecture.md](architecture.md).

## Starting

```bash
cp .env.example .env    # LOGDOCK_PUBLIC_URL at least
mkdir -p data           # owned by PUID:PGID from .env (default 1000:1000)
docker compose up -d --build
docker compose logs logdock | grep setup_code
```

With the image CI publishes, `compose.ghcr.yaml` takes the place of
`compose.yaml`: copied to the server as `compose.yaml`, beside `.env`, it
pulls `ghcr.io/mavnezz/logdock` (`LOGDOCK_IMAGE` pins a version), which is
public: no `docker login` is needed. `docker compose pull && docker compose
up -d` updates it.

Open `LOGDOCK_PUBLIC_URL/setup`, enter the setup code from the log and create the
first administrator. The code is only written while there is no account; set
`LOGDOCK_SETUP_CODE` for automated setups.

`GET /health/live` answers as long as the process runs; `GET /health/ready`
answers 503 with its reasons when the database does not answer, free disk is
below `LOGDOCK_MIN_FREE_DISK` or a writer stalled. The image's `HEALTHCHECK`
asks `/health/live`.

## Reached directly

Compose publishes port 8080 (`LOGDOCK_PORT`) on all the host's addresses;
`LOGDOCK_BIND=127.0.0.1` limits it to a proxy on the same host.

Reached without a proxy, over plain HTTP, `LOGDOCK_PUBLIC_URL` is that
`http://` address (`http://docker-host:8080`). With `https://` or none,
Logdock sets secure cookies, which browsers keep over HTTPS only (and from
`localhost`): the setup and sign-in then answer `insecure_connection`
instead of a session the browser would drop.

## Behind the reverse proxy

Logdock speaks plain HTTP on one port; the proxy terminates TLS. It needs:

- the original `Host` (or `X-Forwarded-Host`), for the check that requests that
  change something come from Logdock's own pages;
- `X-Forwarded-For`, believed from `LOGDOCK_TRUSTED_PROXIES` only;
- no buffering for `/api/v1/logs/stream` (Server-Sent Events). Logdock sends
  `X-Accel-Buffering: no`, which Nginx honours; Caddy and Traefik stream by
  default.
- request bodies up to `LOGDOCK_INGEST_MAX_BODY` (1 MiB by default) on
  `/api/*/envelope/` and `/api/v1/ingest/logs`.

Nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_http_version 1.1;
    proxy_read_timeout 1h;
    client_max_body_size 2m;
}
```

Caddy:

```caddyfile
logdock.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

## E-mail

Logdock sends e-mail about new issues and issues that come back after being
resolved, to the addresses in each project's settings. It needs an SMTP server
in `LOGDOCK_SMTP_URL`:

```bash
LOGDOCK_SMTP_URL=smtp://logdock%40example.com:app-password@mail.example.com:587
LOGDOCK_MAIL_FROM=Logdock <logdock@example.com>
```

`smtp://` upgrades with STARTTLS when the server offers it (`?tls=required`
insists); `smtps://` speaks TLS from the start (port 465). Characters such as
`@` in the user or password are percent-encoded. Without the variable Logdock
logs at the start that it sends no e-mail.

The project's settings have *Send a test e-mail*. A failure there shows what
the mail server answered; failures while notifying are logged as warnings and
not retried.

## Data directory

```text
data/
├── logdock.db              accounts, projects, keys, issues, counts, segment catalog
└── segments/
    ├── logs/2026-11-18.db     one file per day of receipt
    ├── logs/2026-11-18.3d.db  the same day of stages that keep their logs 3 days
    ├── events/…
    └── traces/…
```

A day's segment is written to until the day is over; within the next hour it is
closed (its index merged) and never changes again. Retention deletes whole
segment files (`LOGDOCK_RETENTION_*_DAYS`); above `LOGDOCK_MAX_STORAGE` the oldest
days go first.

A stage can keep its logs, errors or traces for days of its own (a project's
*Settings*, *Retention per stage*): its data goes into files of their own,
`<day>.<days>d.db`, deleted when those days are over. A changed setting applies
to data received from then on, within half a minute; what came before stays as
long as it was going to.

The *Storage* page shows what each day, kind, project and stage takes (each
stage's share measured every hour), the free disk, and where the data is heading
at the rate of the last full days: what it settles at, and in how many days the
storage limit or the disk's reserve (`LOGDOCK_MIN_FREE_DISK`) would be reached.

## Backup

Logdock has no backup of its own; the host's backup saves the data directory.
SQLite runs in WAL mode, so copying a database while it is written is not safe.
Two ways that are:

**Stopped, everything.** Simple and complete; ingest is refused while it is
down, and the SDKs drop what they would have sent.

```bash
docker compose stop logdock
tar -C /srv/logdock -czf /backup/logdock-$(date +%F).tar.gz data
docker compose start logdock
```

**Running.** The closed segments do not change any more and are copied as
files; the main database and today's open segments are copied with SQLite's
backup, which is consistent while they are written (needs `sqlite3` on the
host, 3.27 or newer for `VACUUM INTO`):

```bash
set -euo pipefail
src=/srv/logdock/data
dst=/backup/logdock/$(date +%F)
mkdir -p "$dst/segments"
sqlite3 "$src/logdock.db" "VACUUM INTO '$dst/logdock.db'"
today=$(date -u +%F)
for kind in logs events traces; do
  [ -d "$src/segments/$kind" ] || continue
  mkdir -p "$dst/segments/$kind"
  for file in "$src/segments/$kind"/*.db; do
    name=$(basename "$file")
    case "$name" in
      # Today's, also those of stages' own retention (2026-11-20.3d.db).
      "$today".*) sqlite3 "$file" "VACUUM INTO '$dst/segments/$kind/$name'" ;;
      *) cp -p "$file" "$dst/segments/$kind/$name" ;;
    esac
  done
done
```

Closed segments can also be synchronised incrementally (`rsync`), as they never
change after their day.

## Restore

```bash
docker compose stop logdock
mv /srv/logdock/data /srv/logdock/data.before-restore
mkdir /srv/logdock/data
tar -C /srv/logdock -xzf /backup/logdock-2026-11-18.tar.gz   # or copy a running backup's folder to data/
chown -R 1000:1000 /srv/logdock/data                          # PUID:PGID
docker compose start logdock
```

Logdock refuses to start on a database written by a newer release rather than
guessing at it; restore with the release that wrote the backup, or newer.

## Upgrading

`git pull && docker compose up -d --build`. Migrations run at the start; the
data directory stays as it is.
