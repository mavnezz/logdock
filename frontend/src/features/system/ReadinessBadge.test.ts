import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, reply, stubApi } from '@/shared/testing'

import ReadinessBadge from './ReadinessBadge.vue'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('the readiness badge', () => {
  it('says Logdock is ready', async () => {
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }) })
    const { wrapper } = mountWith(ReadinessBadge)
    await flushPromises()
    expect(wrapper.text()).toContain('Ready to take data')
  })

  it('names the problems when it is not', async () => {
    stubApi({
      'GET /health/ready': () => reply(503, { status: 'not_ready', problems: ['low_disk', 'odd'] }),
    })
    const { wrapper } = mountWith(ReadinessBadge)
    await flushPromises()
    expect(wrapper.text()).toContain('Not ready')
    expect(wrapper.text()).toContain('The data directory is almost full.')
    expect(wrapper.text()).toContain('odd')
  })
})
