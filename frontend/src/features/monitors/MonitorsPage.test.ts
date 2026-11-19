import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { project, session } from '@/shared/testing/fixtures'

import type { Monitor } from './api'
import MonitorPage from './MonitorPage.vue'
import MonitorsPage from './MonitorsPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
  document.body.innerHTML = ''
})

function monitor(overrides: Partial<Monitor> = {}): Monitor {
  return {
    id: 4,
    project_id: 3,
    environment_id: 1,
    slug: 'scheduled_nightly-report',
    name: 'nightly-report',
    schedule: '0 3 * * *',
    timezone: 'Europe/Berlin',
    margin_minutes: 2,
    max_runtime_minutes: 30,
    health: 'failing',
    last_check_in: '2026-11-19T02:00:09Z',
    last_status: 'error',
    next_run: '2026-11-20T02:00:00Z',
    created_at: '2026-11-18T02:00:00Z',
    ...overrides,
  }
}

describe('the scheduled tasks', () => {
  it('lists the tasks with their health and schedule', async () => {
    stubApi({
      'GET /api/v1/monitors': () => [
        monitor(),
        monitor({ id: 5, name: 'heartbeat', health: 'ok', schedule: '* * * * *', timezone: 'UTC' }),
      ],
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper } = mountWith(MonitorsPage)
    await flushPromises()
    expect(wrapper.text()).toContain('nightly-report')
    expect(wrapper.text()).toContain('failing')
    expect(wrapper.text()).toContain('0 3 * * *')
    expect(wrapper.text()).toContain('Europe/Berlin')
    expect(wrapper.find('a[href="/monitors/4"]').exists()).toBe(true)
  })

  it('says how to send check-ins when there are none', async () => {
    stubApi({ 'GET /api/v1/monitors': () => [], 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(MonitorsPage)
    await flushPromises()
    expect(wrapper.text()).toContain('SENTRY_CRONS_MONITOR_SCHEDULED_TASKS')
  })

  it('shows a task’s runs and lets administrators forget it', async () => {
    const calls = stubApi({
      'GET /api/v1/monitors/4': () => ({
        monitor: monitor(),
        check_ins: [
          {
            check_in_id: 'b',
            status: 'error',
            started: '2026-11-19T02:00:02Z',
            duration_ms: 7000,
            release: 'dev-1',
            trace_id: 'abc',
          },
          { check_in_id: 'missed-1', status: 'missed', started: '2026-11-18T02:00:00Z' },
          { check_in_id: 'a', status: 'ok', started: '2026-11-17T02:00:01Z', duration_ms: 30_000 },
        ],
      }),
      'DELETE /api/v1/monitors/4': () => reply(204, undefined),
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper, router } = mountWith(MonitorPage, {
      props: { id: '4' },
      attachTo: document.body,
    })
    useSessionStore().set(session())
    await flushPromises()
    expect(wrapper.text()).toContain('scheduled_nightly-report')
    expect(wrapper.text()).toContain('2 min late, 30 min long')
    expect(wrapper.text()).toContain('7 s')
    expect(wrapper.text()).toContain('missed')
    expect(wrapper.find('a[href="/traces/abc"]').exists()).toBe(true)
    expect(wrapper.findAll('[role="img"] span').length).toBe(3)

    ;[...document.body.querySelectorAll('button')]
      .find((b) => b.textContent?.trim() === 'Delete')!
      .click()
    await flushPromises()
    ;[...document.body.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === 'Delete')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flushPromises()
    expect(calls.calls.some((c) => c.method === 'DELETE')).toBe(true)
    expect(router.currentRoute.value.path).toBe('/monitors')
  })
})
