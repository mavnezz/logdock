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
    expect(api.calls.map((c) => `${c.method} ${new URL(c.url).pathname}`)).toEqual([
      'PATCH /api/v1/me',
      'POST /api/v1/me/password',
    ])
    expect(wrapper.text()).toContain('Passwort')
    await wrapper.find('#profile-theme').setValue('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })
})
