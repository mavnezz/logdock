import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { session } from '@/shared/testing/fixtures'

import AppShell from './AppShell.vue'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('the app shell', () => {
  it('shows administrators everything and signs out', async () => {
    const api = stubApi({
      'GET /health/ready': () => ({ status: 'ready' }),
      'POST /api/v1/auth/logout': () => reply(204, undefined),
    })
    const { wrapper, router } = mountWith(AppShell)
    useSessionStore().set(session())
    await flushPromises()
    expect(wrapper.text()).toContain('Accounts')
    expect(wrapper.text()).toContain('ada@example.com')
    await wrapper.find('button[aria-label="Sign out"]').trigger('click')
    await flushPromises()
    expect(api.calls.some((c) => c.url.endsWith('/api/v1/auth/logout'))).toBe(true)
    expect(router.currentRoute.value.fullPath).toBe('/login')
    expect(useSessionStore().session).toBeNull()
  })

  it('hides the administrators’ pages from members', async () => {
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }) })
    const { wrapper } = mountWith(AppShell)
    useSessionStore().set(session({ role: 'member' }))
    await flushPromises()
    expect(wrapper.text()).not.toContain('Accounts')
    expect(wrapper.text()).toContain('Projects')
    await wrapper.find('button[aria-label="Menu"]').trigger('click')
  })
})
