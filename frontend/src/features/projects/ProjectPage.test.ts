import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { project, session } from '@/shared/testing/fixtures'

import ProjectPage from './ProjectPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

function button(label: string): HTMLButtonElement {
  const found = [...document.body.querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === label,
  )
  if (!found) throw new Error(`no button ${label}`)
  return found
}

const key = {
  id: 7,
  environment_id: 1,
  label: 'web',
  prefix: 'abcd1234',
  created_at: '2026-11-18T09:00:00Z',
  last_used_at: null,
  revoked_at: null,
}

describe('a project’s page', () => {
  it('shows stages and what ingest did', async () => {
    stubApi({
      'GET /api/v1/projects/3': () => project(),
      'GET /api/v1/projects/3/ingest': () => [
        {
          day: '2026-11-18',
          key_id: 7,
          accepted: 1200,
          skipped: 3,
          rejected: { size: 2 },
          last_rejection: { reason: 'size', detail: 'too large', at: '2026-11-18T09:00:00Z' },
        },
        {
          day: '2026-11-18',
          key_id: null,
          accepted: 0,
          skipped: 0,
          rejected: { auth: 1 },
          last_rejection: null,
        },
      ],
      'POST /api/v1/projects/3/environments': () => ({ id: 9, name: 'dev', created_at: '' }),
    })
    const { wrapper } = mountWith(ProjectPage, { props: { id: '3' }, attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    expect(wrapper.text()).toContain('orders-backend')
    expect(wrapper.text()).toContain('staging')
    expect(wrapper.text()).toContain('1,200')
    expect(wrapper.text()).toContain('too large')
    expect(wrapper.text()).toContain('key: 1')
    await wrapper.find('input').setValue('dev')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
  })

  it('creates and revokes keys and shows the DSN once', async () => {
    const api = stubApi({
      'GET /api/v1/projects/3': () => project(),
      'GET /api/v1/projects/3/ingest': () => [],
      'GET /api/v1/projects/3/keys': () => [
        { ...key, id: 9, environment_id: null, label: 'laravel' },
        key,
        { ...key, id: 8, revoked_at: '2026-11-18T10:00:00Z' },
      ],
      'POST /api/v1/projects/3/keys': () => ({
        key: { ...key, environment_id: null },
        secret: 'abcd1234ef',
        dsn: 'https://abcd1234ef@logs.example.com/3',
      }),
      'DELETE /api/v1/projects/3/keys/7': () => reply(204, undefined),
    })
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn(() => Promise.resolve()) } })
    const { wrapper } = mountWith(ProjectPage, { props: { id: '3' }, attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    button('Keys').click()
    await flushPromises()
    expect(wrapper.text()).toContain('abcd1234…')
    expect(wrapper.text()).toContain('revoked')
    const stages = wrapper.findAll('tbody tr').map((row) => row.findAll('td')[1]!.text())
    expect(stages).toEqual(['All stages', 'production', 'production'])

    // A new key serves all stages unless one is chosen.
    button('New key').click()
    await flushPromises()
    document.body.querySelector('#key-label')!.closest('form')!.dispatchEvent(new Event('submit'))
    await flushPromises()
    const posted = api.calls.find((c) => c.method === 'POST')!
    expect(await posted.clone().json()).toEqual({ environment_id: null, label: '' })
    expect(document.body.querySelector<HTMLInputElement>('#dsn')?.value).toBe(
      'https://abcd1234ef@logs.example.com/3',
    )
    expect(document.body.textContent).toContain('The stage: APP_ENV')
    expect(document.body.textContent).not.toContain('SENTRY_ENVIRONMENT=')
    button('Copy').click()
    await flushPromises()

    button('Revoke').click()
    await flushPromises()
    ;[...document.body.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === 'Revoke')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flushPromises()
    expect(api.calls.some((c) => c.method === 'DELETE')).toBe(true)
  })

  it('saves its settings', async () => {
    const api = stubApi({
      'GET /api/v1/projects/3': () => project(),
      'GET /api/v1/projects/3/ingest': () => [],
      'PATCH /api/v1/projects/3': () => project(),
    })
    const { wrapper } = mountWith(ProjectPage, { props: { id: '3' }, attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    button('Settings').click()
    await flushPromises()
    await wrapper.find('#settings-notify').setValue('ops@example.com\nlead@example.com')
    expect((wrapper.find('#settings-overrides').element as HTMLTextAreaElement).value).toBe(
      'GET /health = 0',
    )
    await wrapper.find('#settings-request-ms').setValue('')
    await wrapper.find('#settings-job-ms').setValue('30000')
    await wrapper
      .find('#settings-overrides')
      .setValue('POST /payments = 5000\nnonsense\nGET /health = 0')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    const patch = api.calls.find((c) => c.method === 'PATCH')!
    const body = (await patch.json()) as {
      settings: { notify: string[]; performance: Record<string, unknown> }
    }
    expect(body.settings.notify).toEqual(['ops@example.com', 'lead@example.com'])
    expect(body.settings.performance).toEqual({
      request_ms: null,
      job_ms: 30000,
      slow_query_ms: 1000,
      n_plus_one: true,
      overrides: [
        { name: 'POST /payments', ms: 5000 },
        { name: 'GET /health', ms: 0 },
      ],
    })
    expect(wrapper.text()).toContain('Saved.')
  })

  it('sends a test e-mail and says why it cannot', async () => {
    let configured = false
    stubApi({
      'GET /api/v1/projects/3': () => project(),
      'GET /api/v1/projects/3/ingest': () => [],
      'POST /api/v1/projects/3/test-mail': () =>
        configured ? { recipients: 2 } : reply(409, { code: 'mail_not_configured', status: 409 }),
    })
    const { wrapper } = mountWith(ProjectPage, { props: { id: '3' }, attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    button('Settings').click()
    await flushPromises()
    button('Send a test e-mail').click()
    await flushPromises()
    expect(wrapper.text()).toContain('No mail server is configured')
    configured = true
    button('Send a test e-mail').click()
    await flushPromises()
    expect(wrapper.text()).toContain('Sent to 2 addresses.')
  })

  it('shows members the overview only', async () => {
    stubApi({
      'GET /api/v1/projects/3': () => project(),
      'GET /api/v1/projects/3/ingest': () => [],
    })
    const { wrapper } = mountWith(ProjectPage, { props: { id: '3' } })
    useSessionStore().set(session({ role: 'member' }))
    await flushPromises()
    expect(wrapper.text()).not.toContain('Keys')
    expect(wrapper.find('form').exists()).toBe(false)
  })
})
