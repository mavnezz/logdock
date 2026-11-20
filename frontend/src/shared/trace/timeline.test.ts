import { describe, expect, it } from 'vitest'

import {
  breakdown,
  buildTimeline,
  categoryOf,
  type Node,
  type TraceSpan,
  type TraceTransaction,
  transactionOf,
} from './timeline'

const at = (ms: number) => `2026-11-19T01:08:19.${String(ms * 1000).padStart(6, '0')}Z`

function span(
  id: string,
  parent: string,
  op: string,
  from: number,
  ms: number,
  description = op,
): TraceSpan {
  return {
    span_id: id,
    parent_span_id: parent,
    op,
    description,
    start: at(from),
    duration_us: ms * 1000,
    status: null,
    data: {},
  }
}

function transaction(overrides: Partial<TraceTransaction> = {}): TraceTransaction {
  return {
    project_id: 3,
    environment_id: 1,
    span_id: 'root',
    parent_span_id: null,
    kind: 'request',
    name: 'GET /orders/{order}',
    op: 'http.server',
    status: 'ok',
    http_status: 200,
    failed: false,
    start: at(0),
    duration_us: 100_000,
    service: 'web',
    host: 'dev-host',
    spans: [
      span('boot', 'root', 'app.bootstrap', 0, 10),
      span('mw', 'root', 'middleware.handle', 10, 85),
      span('route', 'mw', 'http.route', 12, 80, 'OrderController@show'),
      ...[0, 1, 2, 3].map((n) =>
        span(
          `q${n}`,
          'route',
          'db.sql.query',
          20 + n * 5,
          4,
          'select * from items where order_id = ?',
        ),
      ),
      span('cache', 'route', 'cache.get', 45, 1, 'rates'),
      span('http', 'route', 'http.client', 50, 30, 'GET https://rates.example.com'),
    ],
    ...overrides,
  }
}

const labels = (nodes: Node[]) => nodes.map((n) => n.label)

describe('the timeline', () => {
  it('puts spans under their parents and runs of one query in a group', () => {
    const { roots, total } = buildTimeline([transaction()])
    expect(total).toBe(100_000)
    const [request] = roots
    expect(request!.kind).toBe('transaction')
    expect(request!.detail).toBe('web · dev-host')
    expect(labels(request!.children)).toEqual(['app.bootstrap', 'middleware.handle'])
    const route = request!.children[1]!.children[0]!
    expect(labels(route.children)).toEqual([
      'select * from items where order_id = ?',
      'rates',
      'GET https://rates.example.com',
    ])
    const queries = route.children[0]!
    expect(queries.kind).toBe('group')
    expect(queries.count).toBe(4)
    expect(queries.duration).toBe(19_000)
    // The controller's own time: 80 ms less what its children covered.
    expect(route.self).toBe(80_000 - 4 * 4_000 - 1_000 - 30_000)
  })

  it('says where the time went, adding up to the request', () => {
    const { roots } = buildTimeline([transaction()])
    const parts = Object.fromEntries(breakdown(roots[0]!).map((p) => [p.category, p.micros / 1000]))
    expect(parts).toEqual({ db: 16, http: 30, cache: 1, framework: 15, app: 38 })
    expect(Object.values(parts).reduce((a, b) => a + b, 0)).toBe(100)
  })

  it('hangs a queued job under the span that queued it, and finds an error’s request', () => {
    const request = transaction({
      spans: [span('publish', 'root', 'queue.publish', 10, 2, 'App\\Jobs\\ChargeOrder')],
    })
    const job = transaction({
      span_id: 'job',
      parent_span_id: 'publish',
      name: 'App\\Jobs\\ChargeOrder',
      op: 'queue.process',
      start: at(50),
      duration_us: 40_000,
      spans: [span('jq', 'job', 'db.sql.query', 55, 5, 'insert into payments')],
    })
    const timeline = buildTimeline([job, request])
    expect(labels(timeline.roots)).toEqual(['GET /orders/{order}'])
    expect(timeline.roots[0]!.children[0]!.children[0]!.label).toBe('App\\Jobs\\ChargeOrder')
    expect(transactionOf(timeline, 'jq', null)?.label).toBe('App\\Jobs\\ChargeOrder')
    expect(transactionOf(timeline, null, 'App\\Jobs\\ChargeOrder')?.spanId).toBe('job')
    expect(transactionOf(timeline, 'unknown', null)?.spanId).toBe('root')
    expect(transactionOf(buildTimeline([]), null, null)).toBeNull()
  })

  it('survives spans and transactions whose parents go in a circle', () => {
    const looped = transaction({
      spans: [
        span('a', 'b', 'db.sql.query', 1, 1),
        span('b', 'a', 'db.sql.query', 2, 1, 'other'),
        span('c', 'missing', 'cache.get', 3, 1),
      ],
    })
    const { roots } = buildTimeline([looped])
    expect(labels(roots[0]!.children)).toEqual(['db.sql.query', 'other', 'cache.get'])

    const one = transaction({
      span_id: 't1',
      parent_span_id: 's2',
      spans: [span('s1', 't1', 'db.sql.query', 1, 1)],
    })
    const two = transaction({
      span_id: 't2',
      parent_span_id: 's1',
      spans: [span('s2', 't2', 'db.sql.query', 1, 1)],
    })
    expect(buildTimeline([one, two]).roots.length).toBeGreaterThan(0)
    expect(buildTimeline([]).roots).toEqual([])
  })

  it('knows what each operation spends time on', () => {
    expect(categoryOf('db.sql.query')).toBe('db')
    expect(categoryOf('db.transaction')).toBe('db')
    expect(categoryOf('http.client')).toBe('http')
    expect(categoryOf('cache.put')).toBe('cache')
    expect(categoryOf('view.render')).toBe('view')
    expect(categoryOf('queue.publish')).toBe('queue')
    expect(categoryOf('app.bootstrap')).toBe('framework')
    expect(categoryOf('http.route')).toBe('app')
  })
})
