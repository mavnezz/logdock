# API tokens and MCP

Programs read Logdock as an account: scripts through the API, Claude and
other agents through MCP (Model Context Protocol). Both use an API token.

## Tokens

*Profile → API tokens → New token.* The token (`ldt_…`) is shown once; Logdock
keeps its SHA-256 only. It acts for its account:

| Scope | May |
|---|---|
| Read | read everything the account reads: projects, logs, issues, events, traces, performance, monitors |
| Read and triage | also resolve, mute and reopen issues |

A token never manages projects, keys, accounts or other tokens, signs nobody
out and has no session, whatever its account may. The profile shows when each
token was last used; revoking one refuses it at once, and a disabled or
deleted account's tokens stop with it. The audit log names the token:
`ada@example.com (token claude)`.

## API

Every request carries the token; no cookie or CSRF token is needed:

```bash
curl -H "Authorization: Bearer ldt_…" https://logdock.example.com/api/v1/issues
```

The API is described at `/api/v1/openapi.json` (OpenAPI 3.1): a token works
for every operation that reads, and for those marked `token`
(`PATCH /api/v1/issues/{id}`). Anything else answers 403 `token_read_only`.

## MCP

Logdock is an MCP server at `/mcp` (Streamable HTTP, no stream from the
server, no session). For Claude Code:

```bash
claude mcp add --transport http logdock https://logdock.example.com/mcp \
  --header "Authorization: Bearer ldt_…"
```

The profile shows this line with the token when it is made. `-s user` makes
Logdock available in every project instead of the current one.

| Tool | What it does |
|---|---|
| `list_projects` | projects and their stages; the names the other tools take |
| `search_logs` | log entries, newest first: words, `"phrases"`, `key:value`, level, service, host, trace, `since` (15m, 6h, 2d, 1w) or `from`/`to` |
| `list_issues` | issues by status, words, project and stage |
| `get_issue` | an issue with its latest event: exception chain, frames (the app's with their code), request, tags, breadcrumbs, trace ID |
| `get_event` | one error event by its event ID |
| `get_trace` | everything of one trace ID across containers: transactions with spans, log entries, error events |
| `performance` | requests, jobs, commands, SQL, outgoing requests or cache: count, failures, percentiles, total time |
| `list_releases` | releases, newest first: requests (count, failures, p95), errors, new issues |
| `list_queues` | queues: put in, ran, failed, retries, how long jobs waited and ran |
| `list_monitors` | scheduled tasks: health, last and next run |
| `change_issue` | resolve, resolve in the next release, mute or reopen; only with *Read and triage* |

The tools call the API with the caller's token, so they see and may exactly
what the token may. They take project and stage by name and answer with the
API's JSON, without what an agent does not need (the code around library
frames, empty fields, IDs where names say more).

Behind the reverse proxy, `/mcp` needs nothing special: the requests are
short, and Nginx and Caddy pass `Authorization` on.
