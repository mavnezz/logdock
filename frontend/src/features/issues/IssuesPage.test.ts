import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import { listed } from './fixtures'
import IssuesPage from './IssuesPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

describe('the issues page', () => {
  it('lists issues and filters them through the URL', async () => {
    const api = stubApi({
      'GET /api/v1/issues': () => [
        listed(),
        listed({ id: 6, title: 'QueryException: Deadlock', status: 'muted', level: 'warning' }),
        listed({ id: 7, title: 'TimeoutException', regressed_at: '2026-11-18T09:00:00Z' }),
      ],
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper, router } = mountWith(IssuesPage, { path: '/issues' })
    await flushPromises()
    expect(wrapper.text()).toContain('PaymentFailed: Payment for order 42 failed')
    expect(wrapper.text()).toContain('orders-backend')
    expect(wrapper.text()).toContain('Muted')
    expect(wrapper.text()).toContain('came back')
    expect(wrapper.find('a[href="/issues/5"]').exists()).toBe(true)
    const first = new URL(api.calls.find((c) => c.url.includes('/issues'))!.url)
    expect(first.searchParams.get('status')).toBe('unresolved')

    await wrapper.find('input[aria-label="Search the issues"]').setValue('payment')
    await wrapper.find('form').trigger('submit')
    await flushPromises()
    expect(router.currentRoute.value.query.q).toBe('payment')

    await wrapper.find('select[aria-label="Status"]').setValue('all')
    await flushPromises()
    expect(router.currentRoute.value.query.status).toBe('all')
    await wrapper.find('select[aria-label="Order"]').setValue('count')
    await flushPromises()
    const last = new URL(api.calls.at(-1)!.url)
    expect(last.searchParams.get('sort')).toBe('count')
    expect(last.searchParams.get('q')).toBe('payment')
  })

  it('pages through issues, and starts over when a filter changes', async () => {
    const api = stubApi({
      'GET /api/v1/issues': (request) => {
        const offset = Number(new URL(request.url).searchParams.get('offset'))
        // 51 on the first page: another follows; 3 on the second.
        const count = offset === 0 ? 51 : 3
        return Array.from({ length: count }, (_, n) =>
          listed({ id: offset + n + 1, title: `Issue ${offset + n + 1}` }),
        )
      },
      'GET /api/v1/projects': () => [project()],
    })
    const { wrapper, router } = mountWith(IssuesPage, { path: '/issues' })
    await flushPromises()
    expect(wrapper.findAll('li')).toHaveLength(50)
    const next = wrapper.findAll('button').find((b) => b.text().includes('Next'))!
    await next.trigger('click')
    await flushPromises()
    expect(router.currentRoute.value.query.page).toBe('2')
    const second = new URL(api.calls.at(-1)!.url)
    expect([second.searchParams.get('offset'), second.searchParams.get('limit')]).toEqual([
      '50',
      '51',
    ])
    expect(wrapper.text()).toContain('Issue 51')
    expect(wrapper.text()).toContain('Page 2')
    expect(
      wrapper
        .findAll('button')
        .find((b) => b.text().includes('Next'))!
        .attributes('disabled'),
    ).toBeDefined()

    await wrapper.find('select[aria-label="Status"]').setValue('all')
    await flushPromises()
    expect(router.currentRoute.value.query).toEqual({ status: 'all' })
  })

  it('says when nothing is open', async () => {
    stubApi({ 'GET /api/v1/issues': () => [], 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(IssuesPage, { path: '/issues' })
    await flushPromises()
    expect(wrapper.text()).toContain('No issues')
    expect(wrapper.text()).toContain('Nothing open')
  })
})
