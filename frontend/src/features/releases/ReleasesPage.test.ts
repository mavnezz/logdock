import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import ReleasesPage from './ReleasesPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

function requests(count: number, flagged: number, p95: number) {
  return {
    count,
    flagged,
    total_ms: count * 50,
    average_ms: 50,
    p50_ms: 40,
    p95_ms: p95,
    p99_ms: p95,
    max_ms: p95,
  }
}

describe('the releases page', () => {
  it('compares each release with the one before', async () => {
    stubApi({
      'GET /api/v1/releases': () => [
        {
          release: '1.1',
          first_seen: '2026-11-20T10:00:00.000000Z',
          last_seen: '2026-11-20T11:00:00.000000Z',
          events: 7,
          issues: 2,
          new_issues: 1,
          requests: requests(200, 10, 900),
        },
        {
          release: '1.0',
          first_seen: '2026-11-19T10:00:00.000000Z',
          last_seen: '2026-11-20T09:59:00.000000Z',
          events: 3,
          issues: 1,
          new_issues: 0,
          requests: requests(400, 4, 1_200),
        },
        {
          release: '0.9',
          first_seen: '2026-11-18T10:00:00.000000Z',
          last_seen: '2026-11-19T09:59:00.000000Z',
          events: 1,
          issues: 1,
          new_issues: 1,
          requests: null,
        },
      ],
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper } = mountWith(ReleasesPage, { path: '/releases' })
    await flushPromises()
    const rows = wrapper.findAll('tbody tr')
    expect(rows).toHaveLength(3)
    // 1.1 against 1.0: more failures (worse), a faster p95 (better).
    const newest = rows[0]!.text()
    expect(newest).toContain('5.0 %')
    expect(newest).toContain('+4.0 pp')
    expect(newest).toContain('−300 ms')
    expect(rows[0]!.find('.text-destructive').text()).toContain('+4.0 pp')
    expect(rows[0]!.find('.text-success').text()).toContain('−300 ms')
    expect(rows[0]!.find('a').attributes('href')).toBe('/issues?status=all&release=1.1')
    // Without requests, no comparison.
    expect(rows[1]!.findAll('.text-success, .text-destructive')).toHaveLength(0)
    expect(rows[2]!.text()).toContain('–')
  })

  it('says how releases appear', async () => {
    stubApi({ 'GET /api/v1/releases': () => [], 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(ReleasesPage, { path: '/releases' })
    await flushPromises()
    expect(wrapper.text()).toContain('No release yet')
  })
})
