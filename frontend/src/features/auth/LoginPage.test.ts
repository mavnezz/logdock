import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, reply, stubApi } from '@/shared/testing'

import LoginPage from './LoginPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
})

const session = {
  account: {
    id: 1,
    email: 'ada@example.com',
    name: 'Ada',
    role: 'admin',
    locale: 'en',
    disabled: false,
    created_at: '',
  },
  csrf_token: 'csrf',
  expires_at: '',
}

describe('the sign-in page', () => {
  it('signs in and goes where the visitor wanted to', async () => {
    const api = stubApi({ 'POST /api/v1/auth/login': () => session })
    const { wrapper, router } = mountWith(LoginPage, { path: '/login?next=/projects/3' })
    await router.isReady()
    await wrapper.find('#email').setValue('ada@example.com')
    await wrapper.find('#password').setValue('correct horse')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    const body = (await api.calls[0]!.json()) as { email: string }
    expect(body.email).toBe('ada@example.com')
    expect(router.currentRoute.value.fullPath).toBe('/projects/3')
  })

  it('says why it did not work', async () => {
    stubApi({ 'POST /api/v1/auth/login': () => reply(401, { code: 'invalid_credentials' }) })
    const { wrapper } = mountWith(LoginPage)
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(wrapper.text()).toContain('Wrong e-mail address or password.')
  })
})
