<img src="assets/icon.svg" width="64" height="64" alt="">

# Logdock

Logs, errors and monitoring for Laravel apps in one self-hosted container. Every container of every app and stage
sends to Logdock, and one web interface searches across all of them.

- **Logs:** Laravel's logs from all apps, containers and stages, with full-text search, filters and a live view.
- **Errors:** exceptions grouped into issues, with their state across releases and e-mail on new and returning ones.
- **Monitoring:** requests, jobs, queries, outgoing requests, cache and scheduled tasks.
- **Other logs:** Nginx's and other containers' output through Vector or Fluent Bit, and a buffered mode for Laravel's
  logs.

The apps keep using the Sentry SDK they already have; only the DSN points to Logdock.
[docs/laravel.md](docs/laravel.md) shows how an app is set up, [docs/forwarders.md](docs/forwarders.md) how Nginx's
and other containers' output gets there too, [docs/architecture.md](docs/architecture.md) explains how and why.

> **Status:** all steps of the plan in [docs/architecture.md](docs/architecture.md), 12, are built and checked against a
> Laravel test app; the trial with a real app on a stage is next.

## Quick start

```bash
cp .env.example .env    # set LOGDOCK_PUBLIC_URL
mkdir -p data           # owned by PUID:PGID from .env
docker compose up -d --build
```

Logdock listens on `127.0.0.1:8080` (`LOGDOCK_BIND`, `LOGDOCK_PORT`), behind your reverse proxy, which terminates TLS.
`GET /health/ready` tells whether it can take data.

## Development

You need Rust (as `rust-toolchain.toml` pins it), Node 24 (`.nvmrc`), `just`, `cargo-deny` and `cargo-llvm-cov`
(`scripts/install-tools.sh`).

```bash
just web-install   # frontend dependencies
just dev           # the server on 127.0.0.1:8080, data in dev/data
just web-dev       # the interface on http://localhost:5173
just check         # everything CI checks
just e2e           # Logdock and a Laravel test app in Docker, checked end to end
```

| Path | Content |
|---|---|
| `backend/` | Cargo workspace: a crate per bounded context, `logdock-kernel`, `-infra`, `-api`, `logdockd` |
| `frontend/` | Vue 3 interface |
| `docker/Dockerfile`, `compose.yaml` | the image and how it runs |
| `dev/laravel`, `dev/e2e`, `compose.dev.yaml` | the Laravel test app and the end-to-end check |
| `forwarders/` | Vector and Fluent Bit for containers' output and the buffered mode |
| `docs/` | architecture, Laravel setup, testing, operations |
