import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { project, session } from '@/shared/testing/fixtures'

import { event, issue } from './fixtures'
import IssuePage from './IssuePage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

const hours = Array.from({ length: 14 * 24 }, (_, i) => ({
  hour: new Date(Date.UTC(2026, 10, 5) + i * 3600_000).toISOString(),
  count: i === 14 * 24 - 1 ? 3 : 0,
}))

function api(status: string = 'unresolved') {
  return stubApi({
    'GET /api/v1/issues/5': () => ({
      issue: issue({ status }),
      hours,
      releases: [{ release: '1.0.1', count: 4, first_seen: '', last_seen: '2026-11-18T09:15:02Z' }],
      latest: event(),
    }),
    'GET /api/v1/issues/5/events': () => [
      {
        event_id: 'b1',
        issue_id: 5,
        project_id: 3,
        environment_id: 1,
        time: '2026-11-18T09:15:02Z',
        level: 'error',
        release: '1.0.1',
        message: 'x',
        service: 'web',
        host: 'docker-host-01',
      },
      {
        event_id: 'a1',
        issue_id: 5,
        project_id: 3,
        environment_id: 1,
        time: '2026-11-18T09:10:00Z',
        level: 'error',
        release: '1.0.0',
        message: 'x',
        service: 'horizon',
      },
    ],
    'GET /api/v1/events/a1': () => ({
      issue_id: 5,
      project_id: 3,
      environment_id: 1,
      received: '',
      event: event({ event_id: 'a1', release: '1.0.0', server_name: 'orders-horizon-1' }),
    }),
    'GET /api/v1/issues/5/tags': () => [
      { key: 'service', total: 3, values: [{ value: 'web', count: 3 }] },
      {
        key: 'release',
        total: 3,
        values: [
          { value: '4f2c1e9', count: 2 },
          { value: '1.0.0', count: 1 },
        ],
      },
    ],
    'GET /api/v1/traces/771a43a4192642f0b136d5159a501700': () => ({
      trace_id: '771a43a4192642f0b136d5159a501700',
      transactions: [
        {
          project_id: 3,
          environment_id: 1,
          span_id: 'root',
          parent_span_id: null,
          kind: 'request',
          name: 'POST /orders/{order}/pay',
          op: 'http.server',
          status: 'internal_error',
          http_status: 500,
          failed: true,
          start: '2026-11-18T09:15:00.000000Z',
          duration_us: 120_000,
          service: 'web',
          host: 'docker-host-01',
          spans: [0, 1, 2].map((n) => ({
            span_id: `q${n}`,
            parent_span_id: 'root',
            op: 'db.sql.query',
            description: 'select * from items where order_id = ?',
            start: `2026-11-18T09:15:00.0${n}0000Z`,
            duration_us: 2_000,
            status: null,
            data: { 'db.system': 'mysql' },
          })),
        },
      ],
      logs: [
        {
          id: '1-1',
          time: '2026-11-18T09:15:01Z',
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
      events: [],
    }),
    'PATCH /api/v1/issues/5': () => issue({ status: 'resolved' }),
    'DELETE /api/v1/issues/5': () => reply(204, undefined),
    'GET /api/v1/projects': () => [project()],
  })
}

function button(label: string): HTMLButtonElement {
  const found = [...document.body.querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!found) throw new Error(`no button ${label}`)
  return found
}

describe('an issue’s page', () => {
  it('shows the latest event with its stack, request, breadcrumbs and trace', async () => {
    api()
    const { wrapper } = mountWith(IssuePage, { props: { id: '5' }, attachTo: document.body })
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('PaymentFailed: Payment for order 42 failed')
    expect(text).toContain('orders-backend / production')
    expect(text).toContain('App\\Payments\\Gateway::charge')
    expect(text).toContain('$response = $this->http->post($url);')
    expect(text).toContain('caused by')
    expect(text).toContain('unhandled')
    expect(text).toContain('select * from orders where id = ?')
    expect(text).toContain('Mozilla/5.0')
    expect(text).toContain('service: web')
    expect(text).toContain('Charging order 42')
    expect(text).toContain('3 events')
    expect(wrapper.findAll('rect[data-bar]').length).toBe(84)

    // The request's flow: the repeated query as one row, and where the time went.
    expect(text).toContain('What the request did')
    expect(text).toContain('3×')
    expect(text).toContain('Database')
    // Where its events came from.
    expect(text).toContain('Where it comes from')
    expect(text).toContain('67 %')
    // The request as Sentry keeps it: every header, the body, the cookies' names.
    expect(text).toContain('[redacted]')
    expect(text).toContain('"order": 42')
    expect(text).toContain('laravel_session')

    // Library frames are hidden until asked for.
    expect(text).not.toContain('GuzzleHttp\\Client::send')
    await wrapper.find('input[type="checkbox"]').setValue(false)
    expect(wrapper.text()).toContain('GuzzleHttp\\Client::send')

    await wrapper.find('rect[data-bar]:last-of-type').trigger('mouseenter')
    expect(wrapper.find('[role="tooltip"]').exists()).toBe(true)
  })

  it('shows another event when one is picked', async () => {
    api()
    const { wrapper } = mountWith(IssuePage, { props: { id: '5' }, attachTo: document.body })
    await flushPromises()
    await wrapper
      .findAll('aside button')
      .find((b) => b.text().includes('horizon'))!
      .trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Chosen event')
    expect(wrapper.text()).toContain('orders-horizon-1')
    button('Show the latest').click()
    await flushPromises()
    expect(wrapper.text()).toContain('Latest event')
  })

  it('resolves, mutes and deletes', async () => {
    const calls = api()
    const { wrapper, router } = mountWith(IssuePage, {
      props: { id: '5' },
      attachTo: document.body,
    })
    useSessionStore().set(session())
    await flushPromises()
    button('Resolve').click()
    await flushPromises()
    const patches = () => calls.calls.filter((c) => c.method === 'PATCH')
    expect(await patches()[0]!.clone().json()).toEqual({ action: 'resolve' })
    button('Resolve in next release').click()
    await flushPromises()
    expect(await patches()[1]!.clone().json()).toEqual({ action: 'resolve_next_release' })

    await wrapper.find('select[aria-label="Mute"]').setValue('day')
    await flushPromises()
    expect(await patches()[2]!.clone().json()).toEqual({ action: 'mute', mute_minutes: 1440 })

    button('Delete').click()
    await flushPromises()
    ;[...document.body.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === 'Delete')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flushPromises()
    expect(calls.calls.some((c) => c.method === 'DELETE')).toBe(true)
    expect(router.currentRoute.value.path).toBe('/issues')
  })

  it('offers to reopen what is resolved', async () => {
    const calls = api('resolved')
    mountWith(IssuePage, { props: { id: '5' }, attachTo: document.body })
    await flushPromises()
    expect(() => button('Resolve')).toThrow()
    button('Reopen').click()
    await flushPromises()
    const patch = calls.calls.find((c) => c.method === 'PATCH')!
    expect(await patch.json()).toEqual({ action: 'reopen' })
  })
})
