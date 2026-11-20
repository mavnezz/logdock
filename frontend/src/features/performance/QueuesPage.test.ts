import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import QueuesPage from './QueuesPage.vue'

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

describe('the queues page', () => {
  it('lists queues and shows one over time', async () => {
    const api = stubApi({
      'GET /api/v1/queues': () => [
        {
          name: 'payments',
          published: 120,
          processed: 118,
          failed: 4,
          retries: 6,
          wait: stats(118, 6, 1_800),
          processing: stats(118, 4, 950),
        },
        {
          name: 'emails',
          published: 3,
          processed: 0,
          failed: 0,
          retries: 0,
          wait: null,
          processing: null,
        },
      ],
      'GET /api/v1/performance/series': () => ({
        bucket_seconds: 1800,
        buckets: [{ start: '2026-11-19T00:00:00.000000Z', ...stats(60, 2, 450) }],
        total: stats(60, 2, 450),
      }),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper, router } = mountWith(QueuesPage, { path: '/queues' })
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('payments')
    expect(text).toContain('3.4 %')
    expect(text).toContain('1.8 s')
    expect(text).toContain('–')
    const first = new URL(api.calls.find((c) => c.url.includes('/queues'))!.url)
    expect(first.searchParams.get('kind')).toBeNull()
    expect(first.searchParams.get('from')).not.toBeNull()

    await wrapper.find('tbody tr').trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.queue).toBe('payments')
    expect(wrapper.text()).toContain('Waiting before running')
    const kinds = api.calls
      .filter((c) => c.url.includes('/series'))
      .map((c) => new URL(c.url).searchParams.get('kind'))
    expect(kinds.sort()).toEqual(['queue_process', 'queue_wait'])

    await wrapper.find('select[aria-label="Time range"]').setValue('7d')
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ range: '7d', queue: 'payments' })
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Close')!
      .trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.queue).toBeUndefined()
  })

  it('says how queues appear', async () => {
    stubApi({ 'GET /api/v1/queues': () => [], 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(QueuesPage, { path: '/queues' })
    await flushPromises()
    expect(wrapper.text()).toContain('No queue yet')
  })
})
