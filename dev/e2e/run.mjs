// The end-to-end check (docs/testing.md): Logdock and the Laravel test app
// in Docker, as compose.dev.yaml describes them. Sets Logdock up, creates a
// project and a key, starts the app's web, worker and scheduler containers
// with that key, runs the app's scenarios and checks what arrived.
//
//   node dev/e2e/run.mjs           fresh start, check, stop
//   node dev/e2e/run.mjs --keep    leave everything running afterwards
//
// Starts from nothing every time: Logdock's data and the captures go.

import { execFileSync } from 'node:child_process'
import { mkdirSync, readdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'

const root = new URL('../..', import.meta.url).pathname
const keep = process.argv.includes('--keep')
const LOGDOCK = 'http://localhost:18480'
const LARAVEL = 'http://localhost:18400'
const MAILPIT = 'http://localhost:18425'
const PROJECT = 'orders-test'

function compose(args, env = {}) {
  execFileSync('docker', ['compose', '-f', 'compose.dev.yaml', ...args], {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, PUID: String(process.getuid()), PGID: String(process.getgid()), ...env },
  })
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** Asks `probe` until it returns something, for `seconds`. */
async function eventually(what, probe, seconds = 30) {
  const until = Date.now() + seconds * 1000
  let last
  while (Date.now() < until) {
    try {
      last = await probe()
      if (last) return last
    } catch (error) {
      last = error
    }
    await sleep(500)
  }
  throw new Error(`${what}: not within ${seconds} s (${last instanceof Error ? last.message : 'nothing'})`)
}

/** Logdock's API with a session, as the web interface uses it. */
class Logdock {
  cookie = ''
  csrf = ''

  async call(method, path, body) {
    const headers = { Origin: LOGDOCK, Accept: 'application/json' }
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    if (this.cookie) headers.Cookie = this.cookie
    if (this.csrf) headers['X-CSRF-Token'] = this.csrf
    const response = await fetch(LOGDOCK + path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    const cookie = response.headers.get('set-cookie')
    if (cookie) this.cookie = cookie.split(';')[0]
    const text = await response.text()
    const json = text ? JSON.parse(text) : null
    if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`)
    if (json && typeof json.csrf_token === 'string') this.csrf = json.csrf_token
    return json
  }

  get(path) {
    return this.call('GET', path)
  }
}

const results = []
async function check(name, run) {
  try {
    const detail = await run()
    results.push({ name, ok: true, detail: detail ?? '' })
  } catch (error) {
    results.push({ name, ok: false, detail: error instanceof Error ? error.message : String(error) })
  }
}
function expect(condition, message) {
  if (!condition) throw new Error(message)
}

async function main() {
  console.log('» fresh start')
  compose(['down', '--volumes', '--remove-orphans'])
  for (const dir of ['dev/data/logdock', 'dev/captured']) {
    rmSync(join(root, dir), { recursive: true, force: true })
    mkdirSync(join(root, dir), { recursive: true })
  }
  compose(['up', '--detach', '--build', 'logdock', 'capture', 'mailpit'])
  await eventually('Logdock ready', async () => (await fetch(`${LOGDOCK}/health/ready`)).ok, 120)

  console.log('» setting Logdock up')
  const logdock = new Logdock()
  await logdock.call('POST', '/api/v1/setup', {
    code: 'dev-setup-code',
    email: 'dev@logdock.test',
    name: 'Developer',
    password: 'logdock-dev-password',
  })
  const project = await logdock.call('POST', '/api/v1/projects', {
    name: PROJECT,
    environments: ['production', 'staging'],
  })
  const production = project.environments.find((e) => e.name === 'production')
  const key = await logdock.call('POST', `/api/v1/projects/${project.id}/keys`, {
    environment_id: production.id,
    label: 'laravel',
  })
  await logdock.call('PATCH', `/api/v1/projects/${project.id}`, {
    settings: { ...project.settings, notify: ['ops@logdock.test'] },
  })
  const dsn = `http://${key.secret}@capture:8081/${project.id}`

  console.log('» starting the Laravel app')
  compose(['up', '--detach', '--build', 'web', 'worker', 'scheduler'], { LARAVEL_DSN: dsn })
  await eventually('Laravel up', async () => (await fetch(`${LARAVEL}/up`)).ok, 90)

  console.log('» running the scenarios')
  const scenario = async (name, status = 200) => {
    const response = await fetch(`${LARAVEL}/scenario/${name}`)
    expect(response.status === status, `/scenario/${name} answered ${response.status}`)
  }
  await scenario('log')
  await scenario('fail', 500)
  await scenario('handled')
  await scenario('job')
  await scenario('work')
  await scenario('slow')
  compose(['exec', '-T', 'scheduler', 'php', 'artisan', 'schedule:run'])

  console.log('» checking what arrived')
  const search = (q) =>
    logdock.get(`/api/v1/logs?project=${project.id}&q=${encodeURIComponent(q)}&from=2000-01-01T00:00:00Z`)
  const entry = (q) =>
    eventually(`a log entry for ${q}`, async () => (await search(q)).entries[0])

  await check('a request’s log entries arrive with the container’s metadata', async () => {
    const placed = await entry('placed')
    expect(placed.environment_id === production.id, `stage ${placed.environment_id}`)
    expect(placed.level === 'info', `level ${placed.level}`)
    expect(placed.service === 'web', `service ${placed.service}`)
    expect(placed.host === 'dev-host', `host ${placed.host}`)
    expect(placed.release === 'dev-1', `release ${placed.release}`)
    expect(placed.attributes.order === 42, `attributes ${JSON.stringify(placed.attributes)}`)
    return `message ${JSON.stringify(placed.message)}, template ${JSON.stringify(placed.template)}`
  })
  await check('secrets in attributes are redacted', async () => {
    const slow = await entry('slow')
    expect(slow.attributes.password === '[redacted]', `password ${slow.attributes.password}`)
    expect(slow.level === 'warn', `level ${slow.level}`)
  })
  await check('entries below SENTRY_LOG_LEVEL stay out', async () => {
    await sleep(1000)
    const debug = await search('stays below')
    expect(debug.entries.length === 0, `${debug.entries.length} debug entries`)
  })

  const issueTitled = (text) =>
    eventually(`an issue with ${text}`, async () =>
      (await logdock.get(`/api/v1/issues?project=${project.id}&status=all`)).find((i) =>
        i.title.includes(text),
      ),
    )
  let failed
  await check('an exception becomes an issue with its cause, tags and frames', async () => {
    const issue = await issueTitled('Payment for order 42')
    failed = await logdock.get(`/api/v1/issues/${issue.id}`)
    const event = failed.latest
    expect(event.exceptions.length === 2, `${event.exceptions.length} exceptions`)
    expect(event.exceptions[0].type === 'RuntimeException', `cause ${event.exceptions[0].type}`)
    const tags = Object.fromEntries(event.tags)
    expect(tags.service === 'web' && tags.host === 'dev-host', `tags ${JSON.stringify(tags)}`)
    const frames = event.exceptions[1].frames
    expect(
      frames.some((f) => f.in_app && (f.filename ?? '').includes('routes/web.php')),
      'no in-app frame in routes/web.php',
    )
    expect(event.release === 'dev-1', `release ${event.release}`)
    return `title ${JSON.stringify(issue.title)}, culprit ${JSON.stringify(issue.culprit)}`
  })
  await check('the request’s log entries share the error’s trace', async () => {
    const traceId = failed?.latest?.trace_id
    expect(traceId, 'the event has no trace ID')
    const trace = await logdock.get(`/api/v1/traces/${traceId}`)
    expect(
      trace.logs.some((l) => l.message.includes('Charging order')),
      `trace logs: ${JSON.stringify(trace.logs.map((l) => l.message))}`,
    )
  })
  await check('a reported, handled exception becomes an issue', async () => {
    const issue = await issueTitled('Coupon')
    return issue.title
  })
  await check('a job’s entries and errors name the worker, not web', async () => {
    const queued = await entry('queue')
    expect(queued.service === 'horizon', `service ${queued.service}`)
    const issue = await issueTitled('could not charge order 43')
    const detail = await logdock.get(`/api/v1/issues/${issue.id}`)
    const tags = Object.fromEntries(detail.latest.tags)
    expect(tags.service === 'horizon', `tags ${JSON.stringify(tags)}`)
    return `transaction ${JSON.stringify(detail.latest.transaction)}`
  })
  await check('the scheduler’s entries name the scheduler', async () => {
    const heartbeat = await entry('heartbeat')
    expect(heartbeat.service === 'scheduler', `service ${heartbeat.service}`)
  })
  const lastHour = () =>
    `project=${project.id}&from=${new Date(Date.now() - 3_600_000).toISOString()}&to=${new Date(Date.now() + 60_000).toISOString()}`
  const named = (kind, name) =>
    eventually(`${kind} ${name}`, async () =>
      (await logdock.get(`/api/v1/performance?kind=${kind}&${lastHour()}`)).find((r) => r.name === name),
    )
  await check('requests, jobs and what they spent their time on are measured', async () => {
    const work = await named('request', 'GET /scenario/work')
    const slow = await named('request', 'GET /scenario/slow')
    expect(slow.p95_ms >= 300, `p95 of /scenario/slow: ${slow.p95_ms} ms`)
    const job = await named('job', 'App\\Jobs\\ChargeOrder')
    expect(job.flagged === 1, `the job failed ${job.flagged} times`)
    await named('query', 'select ? as id, ? as total')
    await named('outgoing', 'GET logdock:8080')
    const miss = await named('cache', 'get exchange-rate')
    expect(miss.flagged === 1, `cache misses ${miss.flagged}`)
    const fail = await named('request', 'GET /scenario/fail')
    expect(fail.flagged === 1, 'the 500 is a failure')
    return `GET /scenario/work ${work.count}×, p95 ${work.p95_ms} ms`
  })
  await check('a request’s trace holds the job it queued', async () => {
    const samples = await logdock.get(
      `/api/v1/performance/samples?kind=request&name=${encodeURIComponent('GET /scenario/job')}&${lastHour()}`,
    )
    expect(samples.length > 0, 'no sample')
    const trace = await eventually('the job in the trace', async () => {
      const t = await logdock.get(`/api/v1/traces/${samples[0].trace_id}`)
      return t.transactions.length >= 2 && t
    })
    const kinds = trace.transactions.map((t) => t.kind).join(', ')
    expect(kinds === 'request, job', `kinds ${kinds}`)
  })
  await check('scheduled tasks become monitors; a failing one is mailed', async () => {
    const monitors = await eventually('both monitors', async () => {
      const list = await logdock.get(`/api/v1/monitors?project=${project.id}`)
      return list.length >= 2 && list
    })
    const report = monitors.find((m) => m.name === 'nightly-report')
    const heartbeat = monitors.find((m) => m.name === 'heartbeat')
    expect(report?.health === 'failing', `nightly-report is ${report?.health}`)
    expect(heartbeat?.health === 'ok', `heartbeat is ${heartbeat?.health}`)
    expect(report.schedule === '* * * * *', `schedule ${report.schedule}`)
    const mail = await eventually('the mail', async () => {
      const messages = await (await fetch(`${MAILPIT}/api/v1/messages`)).json()
      return messages.messages.find((m) => m.Subject.includes('nightly-report failed in orders-test / production'))
    })
    return mail.Subject
  })
  await check('new issues are mailed to the project’s addresses', async () => {
    const mail = await eventually('a mail', async () => {
      const messages = await (await fetch(`${MAILPIT}/api/v1/messages`)).json()
      return messages.messages.find((m) => m.Subject.includes('New issue in orders-test / production'))
    })
    return mail.Subject
  })
  await check('ingest refused nothing', async () => {
    const counts = await logdock.get(`/api/v1/projects/${project.id}/ingest`)
    const refused = counts.filter((c) => Object.keys(c.rejected).length > 0)
    expect(refused.length === 0, JSON.stringify(refused))
    const accepted = counts.reduce((sum, c) => sum + c.accepted, 0)
    const skipped = counts.reduce((sum, c) => sum + c.skipped, 0)
    return `${accepted} accepted, ${skipped} skipped`
  })
  await check('the SDK’s envelopes were captured', async () => {
    const files = readdirSync(join(root, 'dev/captured')).filter((f) => f.endsWith('.envelope'))
    const kinds = new Set(files.flatMap((f) => f.replace(/\.envelope$/, '').split('-')[2].split('+')))
    expect(kinds.has('log') && kinds.has('event'), `kinds ${[...kinds].join(', ')}`)
    return `${files.length} envelopes: ${[...kinds].join(', ')}`
  })
}

let crashed = null
try {
  await main()
} catch (error) {
  crashed = error
}
for (const result of results) {
  console.log(`${result.ok ? '✓' : '✗'} ${result.name}${result.detail ? `\n    ${result.detail}` : ''}`)
}
if (crashed) console.error(`✗ ${crashed instanceof Error ? crashed.message : String(crashed)}`)
if (!keep) {
  try {
    compose(['logs', '--no-color', '--tail', '40', 'logdock', 'web', 'worker'])
  } catch {
    // Nothing to show.
  }
  compose(['down', '--volumes', '--remove-orphans'])
} else {
  console.log(`Logdock: ${LOGDOCK} (dev@logdock.test / logdock-dev-password)`)
}
process.exit(crashed || results.some((r) => !r.ok) ? 1 : 0)
