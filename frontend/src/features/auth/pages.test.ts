import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { session } from '@/shared/testing/fixtures'

import ProfilePage from './ProfilePage.vue'
import SetupPage from './SetupPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

describe('the setup page', () => {
  it('creates the first administrator', async () => {
    const api = stubApi({ 'POST /api/v1/setup': () => session() })
    const { wrapper, router } = mountWith(SetupPage, { path: '/setup' })
    await wrapper.find('#code').setValue('k7qx-2m9p')
    await wrapper.find('#name').setValue('Ada')
    await wrapper.find('#email').setValue('ada@example.com')
    await wrapper.find('#password').setValue('correct horse')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(await api.calls[0]!.json()).toMatchObject({
      code: 'k7qx-2m9p',
      email: 'ada@example.com',
    })
    expect(useSessionStore().isAdmin).toBe(true)
    expect(router.currentRoute.value.fullPath).toBe('/projects')
  })

  it('says when the code is wrong', async () => {
    stubApi({ 'POST /api/v1/setup': () => reply(403, { code: 'invalid_setup_code' }) })
    const { wrapper } = mountWith(SetupPage)
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(wrapper.text()).toContain('The setup code is wrong.')
  })
})

describe('the profile page', () => {
  it('changes name, language and password', async () => {
    const api = stubApi({
      'PATCH /api/v1/me': () => session({ name: 'Ada L.', locale: 'de' }).account,
      'POST /api/v1/me/password': () => reply(204, undefined),
    })
    const { wrapper } = mountWith(ProfilePage)
    useSessionStore().set(session())
    await flushPromises()
    await wrapper.find('#profile-name').setValue('Ada L.')
    await wrapper.find('#profile-locale').setValue('de')
    await wrapper.findAll('form')[0]!.trigger('submit')
    await flushPromises()
    expect(useSessionStore().account?.locale).toBe('de')

    await wrapper.find('#current-password').setValue('correct horse')
    await wrapper.find('#new-password').setValue('a new password')
    await wrapper.findAll('form')[1]!.trigger('submit')
    await flushPromises()
    const changes = api.calls.filter((c) => c.method !== 'GET')
    expect(changes.map((c) => `${c.method} ${new URL(c.url).pathname}`)).toEqual([
      'PATCH /api/v1/me',
      'POST /api/v1/me/password',
    ])
    expect(wrapper.text()).toContain('Passwort')
    await wrapper.find('#profile-theme').setValue('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('makes API tokens and shows how Claude uses them, once', async () => {
    const token = {
      id: 4,
      name: 'claude',
      scope: 'read',
      prefix: 'ldt_abcd1234',
      created_at: '2026-11-20T09:00:00Z',
      last_used_at: null,
      revoked_at: null,
    }
    const api = stubApi({
      'GET /api/v1/me/tokens': () => [
        token,
        { ...token, id: 3, name: 'old', scope: 'triage', revoked_at: '2026-11-19T09:00:00Z' },
      ],
      'POST /api/v1/me/tokens': () => ({
        token,
        secret: 'ldt_abcd1234ef',
        mcp_url: 'https://logs.example.com/mcp',
      }),
      'DELETE /api/v1/me/tokens/4': () => reply(204, undefined),
    })
    const writeText = vi.fn(() => Promise.resolve())
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { wrapper } = mountWith(ProfilePage, { attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    expect(wrapper.text()).toContain('ldt_abcd1234…')
    expect(wrapper.text()).toContain('Read and triage')
    expect(wrapper.text()).toContain('revoked')

    const button = (label: string) =>
      [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === label)!
    button('New token').click()
    await flushPromises()
    const name = document.body.querySelector<HTMLInputElement>('#token-name')!
    name.value = 'claude'
    name.dispatchEvent(new Event('input'))
    const scope = document.body.querySelector<HTMLSelectElement>('#token-scope')!
    scope.value = 'triage'
    scope.dispatchEvent(new Event('change'))
    name.closest('form')!.dispatchEvent(new Event('submit'))
    await flushPromises()
    const posted = api.calls.find((c) => c.method === 'POST')!
    expect(await posted.clone().json()).toEqual({ name: 'claude', scope: 'triage' })
    expect(document.body.querySelector<HTMLInputElement>('#token-secret')?.value).toBe(
      'ldt_abcd1234ef',
    )
    expect(document.body.textContent).toContain(
      'claude mcp add --transport http logdock https://logs.example.com/mcp --header "Authorization: Bearer ldt_abcd1234ef"',
    )
    expect(document.body.textContent).toContain('https://logs.example.com/api/v1/issues')
    button('Copy').click()
    await flushPromises()
    expect(writeText).toHaveBeenCalled()

    button('Revoke').click()
    await flushPromises()
    ;[...document.body.querySelectorAll('[role="alertdialog"] button')]
      .find((b) => b.textContent?.trim() === 'Revoke')!
      .dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await flushPromises()
    expect(api.calls.some((c) => c.method === 'DELETE')).toBe(true)
  })
})
