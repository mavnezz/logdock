import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import type { LogEntry } from './api'
import LogsPage from './LogsPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

function entry(overrides: Partial<LogEntry> = {}): LogEntry {
  return {
    id: '20410-1',
    time: '2026-11-18T09:15:02.123456Z',
    received: '2026-11-18T09:15:02.200000Z',
    project_id: 3,
    environment_id: 1,
    level: 'warn',
    message: 'Payment declined for order 42',
    template: 'Payment declined for order {id}',
    service: 'horizon',
    host: 'docker-host-02',
    container: 'orders-horizon-1',
    release: '4f2c1e9',
    trace_id: '5b8efff798038103d269b633813fc60c',
    span_id: null,
    user_id: null,
    attributes: { order_id: 42, password: '[redacted]' },
    redactions: 1,
    truncated: false,
    ...overrides,
  }
}

const histogram = {
  bucket_seconds: 1800,
  buckets: [
    {
      start: '2026-11-18T09:00:00.000000Z',
      trace: 0,
      debug: 1,
      info: 5,
      warn: 2,
      error: 1,
      fatal: 0,
    },
    {
      start: '2026-11-18T09:30:00.000000Z',
      trace: 0,
      debug: 0,
      info: 0,
      warn: 0,
      error: 0,
      fatal: 0,
    },
  ],
}

class FakeEventSource {
  static last: FakeEventSource | null = null
  listeners: Record<string, (event: { data: string }) => void> = {}
  closed = false
  constructor(readonly url: string) {
    FakeEventSource.last = this
  }
  addEventListener(name: string, listener: (event: { data: string }) => void) {
    this.listeners[name] = listener
  }
  close() {
    this.closed = true
  }
}

function api() {
  return stubApi({
    'GET /api/v1/logs': () => ({
      entries: [entry(), entry({ id: '20410-2', level: 'info', message: 'Job done' })],
      next: 'c1',
    }),
    'GET /api/v1/logs/histogram': () => histogram,
    'GET /api/v1/logs/sources': () => [
      {
        project_id: 3,
        environment_id: 1,
        service: 'horizon',
        host: 'docker-host-02',
        last_seen: '',
      },
    ],
    'GET /api/v1/projects': () => [project()],
  })
}

describe('the logs page', () => {
  it('lists entries, filters them through the URL and opens one', async () => {
    const calls = api()
    const { wrapper, router } = mountWith(LogsPage, { path: '/logs?level=warn' })
    await flushPromises()
    expect(wrapper.text()).toContain('Payment declined for order 42')
    expect(wrapper.text()).toContain('Entries by level')
    const searches = calls.calls.filter((c) => new URL(c.url).pathname === '/api/v1/logs')
    expect(searches.some((c) => new URL(c.url).searchParams.get('level') === 'warn')).toBe(true)

    await wrapper.find('input[aria-label="Search the logs"]').setValue('order_id:42')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(router.currentRoute.value.query.q).toBe('order_id:42')

    await wrapper.find('ol li').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('1 values redacted')
    expect(wrapper.text()).toContain('orders-horizon-1')
    expect(wrapper.text()).toContain('orders-backend / production')
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('Same trace'))!
      .trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.trace).toBe('5b8efff798038103d269b633813fc60c')

    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('Older entries'))!
      .trigger('click')
    await flushPromises()
    const older = calls.calls.filter((c) => new URL(c.url).searchParams.get('before') === 'c1')
    expect(older.length).toBe(1)
  })

  it('narrows the range to a bar of the chart and toggles levels', async () => {
    api()
    const { wrapper, router } = mountWith(LogsPage, { path: '/logs' })
    await flushPromises()
    const bar = wrapper.find('rect[data-bucket]')
    await bar.trigger('mouseenter')
    expect(wrapper.find('[role="tooltip"]').text()).toContain('Warning')
    await bar.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.range).toBe('custom')
    expect(router.currentRoute.value.query.from).toBe('2026-11-18T09:00:00.000Z')

    await wrapper.findAll('button[aria-pressed]')[4]!.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.level).toBe('error')
  })

  it('shows new entries live', async () => {
    api()
    vi.stubGlobal('EventSource', FakeEventSource)
    const { wrapper } = mountWith(LogsPage, { path: '/logs?q=payment' })
    await flushPromises()
    await wrapper
      .findAll('button')
      .find((b) => b.text().trim() === 'Live')!
      .trigger('click')
    await flushPromises()
    const source = FakeEventSource.last!
    expect(source.url).toContain('q=payment')
    expect(source.url).not.toContain('from=')
    source.listeners.entry!({
      data: JSON.stringify(entry({ id: 'live-1', message: 'Just arrived' })),
    })
    source.listeners.lagged!({ data: '3' })
    await flushPromises()
    expect(wrapper.find('ol li').text()).toContain('Just arrived')
    expect(wrapper.text()).toContain('skipped 3 entries')
    wrapper.unmount()
    expect(source.closed).toBe(true)
  })
})
