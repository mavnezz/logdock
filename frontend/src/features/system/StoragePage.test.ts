import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, stubApi } from '@/shared/testing'
import { project, session } from '@/shared/testing/fixtures'

import StoragePage from './StoragePage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
})

const GIB = 1024 ** 3

function storage(overrides: Record<string, unknown> = {}) {
  return {
    kinds: [
      { kind: 'logs', bytes: 3 * GIB, segments: 4, oldest: '2026-11-17', retention_days: 30 },
      { kind: 'events', bytes: GIB / 2, segments: 2, oldest: '2026-11-19', retention_days: 90 },
      { kind: 'traces', bytes: GIB, segments: 2, oldest: '2026-11-19', retention_days: 7 },
    ],
    segments: [
      { day: '2026-11-19', kind: 'logs', retention_days: 30, own_retention: false, bytes: GIB },
      { day: '2026-11-19', kind: 'logs', retention_days: 3, own_retention: true, bytes: GIB / 4 },
      { day: '2026-11-19', kind: 'traces', retention_days: 7, own_retention: false, bytes: GIB },
      { day: '2026-11-20', kind: 'logs', retention_days: 30, own_retention: false, bytes: GIB },
      {
        day: '2026-11-20',
        kind: 'events',
        retention_days: 90,
        own_retention: false,
        bytes: GIB / 2,
      },
    ],
    stages: [
      { project_id: 3, environment_id: 1, kind: 'logs', rows: 900_000, bytes: 2 * GIB },
      { project_id: 3, environment_id: 1, kind: 'traces', rows: 40_000, bytes: GIB },
      { project_id: 3, environment_id: 2, kind: 'logs', rows: 100_000, bytes: GIB / 4 },
    ],
    forecast: {
      days: 1,
      daily_bytes: 2 * GIB,
      settled_bytes: 70 * GIB,
      limit_in_days: 23,
      disk_in_days: 9,
    },
    segment_bytes: 4.5 * GIB,
    max_storage_bytes: 50 * GIB,
    database_bytes: 64 * 1024 ** 2,
    free_bytes: 20 * GIB,
    min_free_bytes: 2 * GIB,
    rollups_days: 90,
    ...overrides,
  }
}

const staging = { environment_id: 2, logs_days: 3, events_days: null, traces_days: null }

describe('the storage page', () => {
  it('shows what the data takes, by day, kind and stage, and where it is heading', async () => {
    const base = project()
    stubApi({
      'GET /api/v1/system/storage': () => storage(),
      'GET /api/v1/projects': () => [
        project({ settings: { ...base.settings, retention: [staging] } }),
      ],
    })
    const { wrapper } = mountWith(StoragePage, { path: '/storage' })
    useSessionStore().set(session())
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('4.6 GiB')
    expect(text).toContain('of it 64 MiB the main database')
    expect(text).toContain('9 % of the limit in use')
    expect(text).toContain('settles at about 70 GiB')
    expect(text).toContain('down to its reserve in 9 days')
    expect(text).toContain('storage limit is reached in 23 days')
    expect(text).toContain('17 Nov 2026')
    // Staging keeps its logs 3 days of its own.
    const rows = wrapper.findAll('table[aria-label="By project and stage"] tbody tr')
    expect(rows[0]!.text()).toContain('orders-backend / production')
    expect(rows[0]!.text()).toContain('30 days')
    expect(rows[1]!.text()).toContain('3 days')
    expect(rows[1]!.text()).toContain('own')
    expect(wrapper.find('a[href="/projects/3?tab=settings"]').exists()).toBe(true)
    expect(text).toContain('1,040,000 entries')

    await wrapper.find('g[data-day]').trigger('mouseenter')
    expect(wrapper.find('[role="tooltip"]').text()).toContain('19 Nov 2026 · 2.3 GiB')
  })

  it('says what is not known yet', async () => {
    stubApi({
      'GET /api/v1/system/storage': () =>
        storage({
          forecast: null,
          stages: [],
          segments: [],
          max_storage_bytes: 0,
          free_bytes: null,
        }),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper } = mountWith(StoragePage, { path: '/storage' })
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('none')
    expect(text).toContain('known after the first full day')
    expect(text).toContain('Not measured yet')
    expect(text).not.toContain('reserve in')
  })
})
