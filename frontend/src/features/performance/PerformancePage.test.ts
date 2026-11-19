import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import PerformancePage from './PerformancePage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
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

function api() {
  return stubApi({
    'GET /api/v1/performance': (request) => {
      const kind = new URL(request.url).searchParams.get('kind')
      if (kind === 'cache') return [{ name: 'get exchange-rate', ...stats(10, 4, 1) }]
      return [
        { name: 'GET /orders/{order}', ...stats(120, 3, 480) },
        { name: 'GET /', ...stats(900, 0, 35) },
      ]
    },
    'GET /api/v1/performance/series': () => ({
      bucket_seconds: 1800,
      buckets: [
        { start: '2026-11-19T00:00:00.000000Z', ...stats(60, 2, 450) },
        { start: '2026-11-19T00:30:00.000000Z', ...stats(0, 0, 0) },
        { start: '2026-11-19T01:00:00.000000Z', ...stats(60, 1, 510) },
      ],
      total: stats(120, 3, 480),
    }),
    'GET /api/v1/performance/samples': () => [
      {
        trace_id: '414460eb7b5c4bc1913034f6998e4ce8',
        project_id: 3,
        environment_id: 1,
        name: 'GET /orders/{order}',
        start: '2026-11-19T01:08:19Z',
        duration_ms: 2300,
        status: 'internal_error',
        http_status: 500,
        failed: true,
        service: 'web',
      },
    ],
    'GET /api/v1/projects': () => [project()],
  })
}

describe('the performance page', () => {
  it('lists names of a kind and opens one with its charts and slowest traces', async () => {
    const calls = api()
    const { wrapper, router } = mountWith(PerformancePage, { path: '/performance' })
    await flushPromises()
    expect(wrapper.text()).toContain('GET /orders/{order}')
    expect(wrapper.text()).toContain('2.5 %')
    expect(wrapper.text()).toContain('480 ms')

    await wrapper.findAll('tbody tr')[0]!.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.name).toBe('GET /orders/{order}')
    expect(wrapper.text()).toContain('Count over time')
    expect(wrapper.text()).toContain('p95 over time')
    expect(wrapper.find('a[href="/traces/414460eb7b5c4bc1913034f6998e4ce8"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('2.3 s')
    await wrapper.find('rect[data-point]').trigger('mouseenter')
    expect(wrapper.find('[role="tooltip"]').text()).toContain('Failed')

    await wrapper
      .findAll('[role="tab"]')
      .find((t) => t.text() === 'Cache')!
      .trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.kind).toBe('cache')
    expect(router.currentRoute.value.query.name).toBeUndefined()
    expect(wrapper.text()).toContain('Misses')
    expect(wrapper.text()).toContain('40 %')

    await wrapper.find('select[aria-label="Order"]').setValue('slowest')
    await flushPromises()
    await wrapper.find('select[aria-label="Time range"]').setValue('7d')
    await flushPromises()
    const last = new URL(calls.calls.filter((c) => c.url.includes('/performance?')).at(-1)!.url)
    expect(last.searchParams.get('sort')).toBe('slowest')
    const span =
      new Date(last.searchParams.get('to')!).getTime() -
      new Date(last.searchParams.get('from')!).getTime()
    expect(span).toBe(7 * 86_400_000)
  })

  it('says how to send data when there is none', async () => {
    stubApi({ 'GET /api/v1/performance': () => [], 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(PerformancePage, { path: '/performance?kind=job' })
    await flushPromises()
    expect(wrapper.text()).toContain('Nothing measured')
    expect(wrapper.text()).toContain('SENTRY_TRACES_SAMPLE_RATE')
  })
})
