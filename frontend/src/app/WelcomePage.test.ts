import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'

import { routes } from './router'
import WelcomePage from './WelcomePage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('the welcome page', () => {
  it('names Logdock and whether it is ready', async () => {
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }) })
    const { wrapper } = mountWith(WelcomePage)
    await flushPromises()
    expect(wrapper.text()).toContain('Logdock')
    expect(wrapper.text()).toContain('Ready to take data')
  })

  it('is where unknown paths lead', () => {
    const fallback = routes.find((route) => route.path.startsWith('/:pathMatch'))
    expect(fallback?.redirect).toBe('/')
  })
})
