import { describe, expect, it } from 'vitest'

import { mountWith } from '@/shared/testing'

import RequestTimeline from './RequestTimeline.vue'
import type { TraceSpan, TraceTransaction } from './timeline'

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
    status: op === 'http.client' ? 'internal_error' : null,
    // The schema types span data as an empty object; the API sends any.
    data: (op.startsWith('db')
      ? { 'db.system': 'mysql', 'code.lineno': 31 }
      : {}) as TraceSpan['data'],
  }
}

const request: TraceTransaction = {
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
    span('route', 'root', 'http.route', 5, 90, 'OrderController@show'),
    ...[0, 1, 2].map((n) =>
      span(`q${n}`, 'route', 'db.sql.query', 10 + n * 5, 4, 'select * from items'),
    ),
    span('http', 'route', 'http.client', 40, 30, 'GET https://rates.example.com'),
    span('publish', 'route', 'queue.publish', 75, 2, 'App\\Jobs\\ChargeOrder'),
  ],
}

const job: TraceTransaction = {
  ...request,
  span_id: 'job',
  parent_span_id: 'publish',
  kind: 'job',
  name: 'App\\Jobs\\ChargeOrder',
  op: 'queue.process',
  start: at(80),
  duration_us: 15_000,
  service: 'horizon',
  spans: [span('jq', 'job', 'db.sql.query', 82, 10, 'insert into payments')],
}

describe('the request timeline', () => {
  it('shows the flow, where the time went, and a span’s details', async () => {
    const { wrapper } = mountWith(RequestTimeline, {
      props: { transactions: [request, job], focusSpan: 'http' },
    })
    const text = () => wrapper.text()
    expect(text()).toContain('GET /orders/{order}')
    expect(wrapper.find('[role="img"]').attributes('aria-label')).toBe('Where the time went')
    expect(text()).toContain('Outgoing requests')
    expect(text()).toContain('3×')
    expect(text()).toContain('the event')
    // The queued job is folded under the span that queued it.
    expect(text()).toContain('App\\Jobs\\ChargeOrder')
    expect(text()).not.toContain('insert into payments')

    const toggles = () => wrapper.findAll('button[aria-expanded]')
    const folded = toggles().find((b) => b.attributes('aria-expanded') === 'false')!
    await folded.trigger('click')
    const stillFolded = toggles().filter((b) => b.attributes('aria-expanded') === 'false')
    for (const button of stillFolded) await button.trigger('click')
    expect(text()).toContain('insert into payments')

    // A span's details, and zooming into it and out again.
    const query = wrapper.findAll('li').find((li) => li.text().includes('insert into payments'))!
    await query.trigger('click')
    expect(text()).toContain('Own time')
    expect(text()).toContain('db.system')
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('Zoom in'))!
      .trigger('click')
    expect(text()).toContain('Whole flow')
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('Whole flow'))!
      .trigger('click')
    expect(text()).toContain('Zoom in')

    // The repeated query: how often, and each on average.
    const group = wrapper.findAll('li').find((li) => li.text().includes('3×'))!
    await group.trigger('click')
    expect(text()).toContain('Each on average')
    expect(group.classes()).toContain('bg-accent')
    await group.trigger('click')
    expect(text()).not.toContain('Each on average')

    // Folding hides what is below.
    await toggles()[0]!.trigger('click')
    expect(text()).not.toContain('select * from items')
  })

  it('opens a transaction by name and shows nothing without data', () => {
    const { wrapper } = mountWith(RequestTimeline, {
      props: { transactions: [request], focusName: 'GET /orders/{order}' },
    })
    expect(wrapper.text()).toContain('App code')
    const empty = mountWith(RequestTimeline, { props: { transactions: [] } })
    expect(empty.wrapper.findAll('li')).toHaveLength(0)
  })
})
