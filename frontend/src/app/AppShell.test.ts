import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, reply, stubApi } from '@/shared/testing'
import { session } from '@/shared/testing/fixtures'

import AppShell from './AppShell.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
  document.body.innerHTML = ''
})

/** A wide screen, and a system without a dark preference. */
function wideScreen() {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('min-width'),
    addEventListener: () => undefined,
  }))
}

const ROUTES = [
  {
    path: '/',
    component: AppShell,
    children: [
      { path: 'issues', component: { template: '<p>list</p>' }, meta: { scoped: true } },
      { path: 'issues/:id', component: { template: '<p>one</p>' } },
      { path: 'traces/:id', component: { template: '<p>trace</p>' } },
      { path: 'projects', component: { template: '<p>projects</p>' } },
    ],
  },
]

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

  it('says where one is, and offers the project and stage where they apply', async () => {
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }), 'GET /api/v1/projects': () => [] })
    const { wrapper, router } = mountWith(
      { template: '<RouterView />' },
      { routes: ROUTES, path: '/issues' },
    )
    useSessionStore().set(session())
    await flushPromises()
    const crumbs = () => wrapper.find('nav[aria-label="Where you are"]').text()
    expect(crumbs()).toContain('Observe')
    expect(crumbs()).toContain('Issues')
    expect(wrapper.find('select[aria-label="Project"]').exists()).toBe(true)
    expect(wrapper.find('a[aria-current="page"]').text()).toBe('Issues')

    await router.push('/issues/12')
    await flushPromises()
    expect(crumbs()).toContain('#12')
    expect(wrapper.find('nav[aria-label="Where you are"] a').attributes('href')).toBe('/issues')
    expect(wrapper.find('select[aria-label="Project"]').exists()).toBe(false)

    await router.push('/traces/0123456789abcdef0123456789abcdef')
    await flushPromises()
    expect(crumbs()).toContain('Trace')
    expect(crumbs()).toContain('01234567')

    await router.push('/projects')
    await flushPromises()
    expect(crumbs()).toContain('Setup')
  })

  it('folds the sidebar to its icons, and remembers that', async () => {
    wideScreen()
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }) })
    const { wrapper } = mountWith(AppShell)
    useSessionStore().set(session())
    await flushPromises()
    await wrapper.find('button[aria-label="Collapse the sidebar"]').trigger('click')
    expect(localStorage.getItem('logdock.sidebar')).toBe('collapsed')
    expect(wrapper.find('a[title="Issues"]').exists()).toBe(true)
    await wrapper.find('button[aria-label="Expand the sidebar"]').trigger('click')
    expect(localStorage.getItem('logdock.sidebar')).toBeNull()
    expect(wrapper.find('a[title="Issues"]').exists()).toBe(false)
  })

  it('switches the appearance in turn, and opens the command menu', async () => {
    wideScreen()
    stubApi({ 'GET /health/ready': () => ({ status: 'ready' }), 'GET /api/v1/projects': () => [] })
    const { wrapper } = mountWith(AppShell, { attachTo: document.body })
    useSessionStore().set(session())
    await flushPromises()
    const theme = () => wrapper.find('button[aria-label^="Appearance"]')
    expect(theme().attributes('aria-label')).toBe('Appearance: Like the system')
    await theme().trigger('click')
    expect(theme().attributes('aria-label')).toBe('Appearance: Light')
    await theme().trigger('click')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    await theme().trigger('click')
    expect(theme().attributes('aria-label')).toBe('Appearance: Like the system')

    await wrapper.find('button[aria-label="Search or jump to…"]').trigger('click')
    await flushPromises()
    expect(document.body.querySelector('input[role="combobox"]')).not.toBeNull()
    wrapper.unmount()
  })
})
