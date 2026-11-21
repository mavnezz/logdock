import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import { overviewParams } from './api'
import OverviewPage from './OverviewPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
  localStorage.clear()
})

function stats(count: number, flagged: number, p95: number) {
  return {
    count,
    flagged,
    total_ms: count * 50,
    average_ms: 50,
    p50_ms: 40,
    p95_ms: p95,
    p99_ms: p95 * 1.5,
    max_ms: p95 * 2,
  }
}

const levels = { trace: 0, debug: 2, info: 40, warn: 3, error: 5, fatal: 1 }

function overview(empty = false) {
  const n = empty ? 0 : 1
  return {
    from: '2026-11-19T12:00:00.000000Z',
    to: '2026-11-20T12:00:00.000000Z',
    requests: {
      bucket_seconds: 1800,
      buckets: [{ start: '2026-11-20T11:30:00Z', count: 1200 * n, failed: 12 * n, p95_ms: 180 }],
      total: stats(1200 * n, 12 * n, 180),
    },
    jobs: {
      bucket_seconds: 1800,
      buckets: [{ start: '2026-11-20T11:30:00Z', count: 30 * n, failed: 2 * n, p95_ms: 900 }],
      total: stats(30 * n, 2 * n, 900),
    },
    slowest_routes: empty ? [] : [{ name: 'POST /checkout', ...stats(80, 4, 2_400) }],
    failing_routes: empty ? [] : [{ name: 'POST /checkout', ...stats(80, 4, 2_400) }],
    errors: {
      bucket_seconds: 3600,
      buckets: [{ start: '2026-11-20T11:00:00Z', count: 9 * n }],
      events: 9 * n,
      new_issues: n,
      regressed: n,
      top: empty
        ? []
        : [
            {
              id: 5,
              project_id: 3,
              environment_id: 1,
              category: 'error',
              title: 'PaymentFailed: Payment for order 42 failed',
              culprit: 'App\\Payments\\Gateway::charge',
              level: 'error',
              status: 'unresolved',
              first_seen: '2026-11-20T08:00:00.000000Z',
              last_seen: '2026-11-20T11:58:00.000000Z',
              count: 12,
              events: 9,
              users: 4,
            },
          ],
    },
    logs: {
      bucket_seconds: 1800,
      buckets: [
        {
          start: '2026-11-20T11:30:00Z',
          ...(empty ? { ...levels, info: 0, debug: 0, warn: 0, error: 0, fatal: 0 } : levels),
        },
      ],
      total: empty ? { trace: 0, debug: 0, info: 0, warn: 0, error: 0, fatal: 0 } : levels,
    },
    monitors: {
      total: n * 2,
      ok: n,
      failing: empty
        ? []
        : [
            {
              id: 7,
              project_id: 3,
              environment_id: 1,
              slug: 'nightly-report',
              name: 'nightly-report',
              health: 'failing',
              created_at: '',
            },
          ],
    },
  }
}

describe('the overview', () => {
  it('sums up requests, jobs, errors, logs and scheduled tasks', async () => {
    const api = stubApi({
      'GET /api/v1/overview': () => overview(),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper, router } = mountWith(OverviewPage, { path: '/overview' })
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('1,200')
    expect(text).toContain('1 % failed')
    expect(text).toContain('180 ms')
    expect(text).toContain('2 failed')
    expect(text).toContain('1 new issue')
    expect(text).toContain('1 came back')
    expect(text).toContain('6') // log errors: error and fatal
    expect(text).toContain('1 / 2')
    expect(text).toContain('1 in trouble')
    // The busiest issue, new in the range, with its users.
    expect(text).toContain('PaymentFailed: Payment for order 42 failed')
    expect(text).toContain('9 events')
    expect(text).toContain('4 users')
    expect(text).toContain('new')
    expect(text).toContain('4 failed (5 %)')
    expect(wrapper.find('a[href="/monitors/7"]').exists()).toBe(true)
    const links = wrapper.findAll('a').map((a) => decodeURIComponent(a.attributes('href')!))
    expect(links).toContain('/performance?name=POST+/checkout')
    expect(links).toContain('/logs?level=error,fatal&range=24h')

    await wrapper.find('select[aria-label="Time range"]').setValue('7d')
    await flushPromises()
    expect(router.currentRoute.value.query.range).toBe('7d')
    const last = new URL(api.calls.at(-1)!.url)
    const from = new Date(last.searchParams.get('from')!)
    const to = new Date(last.searchParams.get('to')!)
    expect(Math.round((to.getTime() - from.getTime()) / 86_400_000)).toBe(7)
    await wrapper.find('select[aria-label="Time range"]').setValue('24h')
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({})
  })

  it('says when nothing came in, and moves on with the time', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    const api = stubApi({
      'GET /api/v1/overview': () => overview(true),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper } = mountWith(OverviewPage, { path: '/overview?range=1h' })
    await flushPromises()
    expect(wrapper.text()).toContain('Nothing in this time range')
    const before = api.calls.filter((c) => c.url.includes('/overview')).length
    await vi.advanceTimersByTimeAsync(61_000)
    await flushPromises()
    expect(api.calls.filter((c) => c.url.includes('/overview')).length).toBeGreaterThan(before)
    wrapper.unmount()
  })

  it('asks for the chosen project and stage', () => {
    const now = new Date('2026-11-20T12:00:00Z')
    expect(overviewParams({ range: '1h', project: 3, environment: 1 }, now)).toEqual({
      from: '2026-11-20T11:00:00.000Z',
      to: '2026-11-20T12:00:00.000Z',
      project: '3',
      environment: '1',
    })
  })
})
