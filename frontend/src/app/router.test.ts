import { createPinia, setActivePinia } from 'pinia'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'

import { reply, stubApi } from '@/shared/testing'

import { installGuard, routes } from './router'

afterEach(() => {
  vi.unstubAllGlobals()
})

function router() {
  const pinia = createPinia()
  setActivePinia(pinia)
  const router = createRouter({ history: createMemoryHistory(), routes })
  installGuard(router, pinia)
  return router
}

const session = (role: string) => ({
  account: {
    id: 1,
    email: 'a@b.de',
    name: 'A',
    role,
    locale: 'en',
    disabled: false,
    created_at: '',
  },
  csrf_token: 'c',
  expires_at: '',
})

describe('the navigation guard', () => {
  it('sends visitors to the setup while there is no account', async () => {
    stubApi({
      'GET /api/v1/auth/session': () => reply(401, { code: 'unauthenticated' }),
      'GET /api/v1/setup': () => ({ needed: true }),
    })
    const r = router()
    await r.push('/projects')
    expect(r.currentRoute.value.name).toBe('setup')
  })

  it('sends visitors to the sign-in, remembering where they wanted to go', async () => {
    stubApi({
      'GET /api/v1/auth/session': () => reply(401, { code: 'unauthenticated' }),
      'GET /api/v1/setup': () => ({ needed: false }),
    })
    const r = router()
    await r.push('/projects/4')
    expect(r.currentRoute.value.name).toBe('login')
    expect(r.currentRoute.value.query.next).toBe('/projects/4')
  })

  it('keeps members out of the administrators’ pages', async () => {
    stubApi({ 'GET /api/v1/auth/session': () => session('member') })
    const r = router()
    await r.push('/accounts')
    expect(r.currentRoute.value.fullPath).toBe('/logs')
    await r.push('/login')
    expect(r.currentRoute.value.fullPath).toBe('/logs')
  })
})
