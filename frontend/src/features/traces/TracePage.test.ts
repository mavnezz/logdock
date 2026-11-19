import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import { type TraceTransaction, waterfall } from './api'
import TracePage from './TracePage.vue'

afterEach(() => vi.unstubAllGlobals())

function transaction(overrides: Partial<TraceTransaction> = {}): TraceTransaction {
  return {
    project_id: 3,
    environment_id: 1,
    span_id: 'root',
    parent_span_id: null,
    kind: 'request',
    name: 'GET /scenario/job',
    op: 'http.server',
    status: 'ok',
    http_status: 200,
    failed: false,
    start: '2026-11-19T01:08:19.000000Z',
    duration_us: 64_000,
    service: 'web',
    host: 'dev-host',
    spans: [
      {
        span_id: 'route',
        parent_span_id: 'root',
        op: 'http.route',
        description: 'Closure',
        start: '2026-11-19T01:08:19.010000Z',
        duration_us: 50_000,
        status: null,
        data: {},
      },
      {
        span_id: 'publish',
        parent_span_id: 'route',
        op: 'queue.publish',
        description: 'App\\Jobs\\ChargeOrder',
        start: '2026-11-19T01:08:19.020000Z',
        duration_us: 17_000,
        status: null,
        data: {},
      },
      {
        span_id: 'orphan',
        parent_span_id: 'elsewhere',
        op: 'db.sql.query',
        description: 'select 1',
        start: '2026-11-19T01:08:19.030000Z',
        duration_us: 1_000,
        status: null,
        data: {},
      },
    ],
    ...overrides,
  }
}

describe('a trace', () => {
  it('lays transactions and spans out under their parents', () => {
    const job = transaction({
      span_id: 'job',
      parent_span_id: 'publish',
      kind: 'job',
      name: 'App\\Jobs\\ChargeOrder',
      op: 'queue.process',
      failed: true,
      start: '2026-11-19T01:08:19.500000Z',
      duration_us: 30_000,
      spans: [],
    })
    const { rows, total } = waterfall([transaction(), job])
    expect(total).toBe(530_000)
    expect(rows.map((r) => [r.depth, r.label])).toEqual([
      [0, 'GET /scenario/job'],
      [1, 'Closure'],
      [2, 'App\\Jobs\\ChargeOrder'],
      [1, 'select 1'],
      [0, 'App\\Jobs\\ChargeOrder'],
    ])
    expect(rows[4]!.offset).toBe(500_000)
    expect(rows[4]!.failed).toBe(true)
    expect(waterfall([]).rows).toEqual([])
  })

  it('shows the waterfall, the errors and the log entries of a trace', async () => {
    stubApi({
      'GET /api/v1/traces/abc': () => ({
        trace_id: 'abc',
        transactions: [transaction()],
        logs: [
          {
            id: '1-1',
            time: '2026-11-19T01:08:19.015000Z',
            received: '',
            project_id: 3,
            environment_id: 1,
            level: 'info',
            message: 'Charging order 42',
            attributes: {},
            redactions: 0,
            truncated: false,
          },
        ],
        events: [
          {
            event_id: 'e1',
            issue_id: 5,
            project_id: 3,
            environment_id: 1,
            time: '2026-11-19T01:08:19Z',
            level: 'error',
            message: 'PaymentFailed: Payment for order 42 failed',
          },
        ],
      }),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper } = mountWith(TracePage, { props: { id: 'abc' } })
    await flushPromises()
    expect(wrapper.text()).toContain('orders-backend / production')
    expect(wrapper.text()).toContain('GET /scenario/job')
    expect(wrapper.text()).toContain('queue.publish')
    expect(wrapper.text()).toContain('Charging order 42')
    expect(wrapper.find('a[href="/issues/5"]').exists()).toBe(true)
    expect(wrapper.find('a[href="/logs?trace=abc"]').exists()).toBe(true)
  })

  it('says when nothing carries it', async () => {
    stubApi({
      'GET /api/v1/traces/gone': () => ({
        trace_id: 'gone',
        transactions: [],
        logs: [],
        events: [],
      }),
    })
    const { wrapper } = mountWith(TracePage, { props: { id: 'gone' } })
    await flushPromises()
    expect(wrapper.text()).toContain('Nothing carries this trace')
  })
})
