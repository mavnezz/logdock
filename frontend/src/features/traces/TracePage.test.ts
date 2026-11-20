import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import type { TraceTransaction } from './api'
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
