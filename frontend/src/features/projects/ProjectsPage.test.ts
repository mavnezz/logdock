import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useSessionStore } from '@/shared/session/store'
import { mountWith, stubApi } from '@/shared/testing'

import { laravelEnv } from './laravel'
import ProjectsPage from './ProjectsPage.vue'

afterEach(() => {
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
})

const project = {
  id: 3,
  name: 'orders-backend',
  created_at: '2026-11-18T09:00:00Z',
  environments: [{ id: 1, name: 'production', created_at: '' }],
  settings: {},
}

describe('the projects page', () => {
  it('lists projects with their stages', async () => {
    stubApi({ 'GET /api/v1/projects': () => [project] })
    const { wrapper } = mountWith(ProjectsPage)
    await flushPromises()
    expect(wrapper.text()).toContain('orders-backend')
    expect(wrapper.text()).toContain('production')
  })

  it('lets administrators create one', async () => {
    const api = stubApi({
      'GET /api/v1/projects': () => [],
      'POST /api/v1/projects': () => project,
    })
    const { wrapper, router } = mountWith(ProjectsPage, { attachTo: document.body })
    useSessionStore().set({
      account: {
        id: 1,
        email: 'a@b.de',
        name: 'A',
        role: 'admin',
        locale: 'en',
        disabled: false,
        created_at: '',
      },
      csrf_token: 'c',
      expires_at: '',
    })
    await flushPromises()
    expect(wrapper.text()).toContain('No project yet')
    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('New project'))!
      .trigger('click')
    await flushPromises()
    const name = document.body.querySelector<HTMLInputElement>('#project-name')!
    name.value = 'orders-backend'
    name.dispatchEvent(new Event('input'))
    document.body.querySelector('form')!.dispatchEvent(new Event('submit'))
    await flushPromises()
    const created = api.calls.find((c) => c.method === 'POST')!
    expect(await created.json()).toEqual({
      name: 'orders-backend',
      environments: ['production', 'staging'],
    })
    expect(router.currentRoute.value.fullPath).toBe('/projects/3')
  })

  it('writes the .env lines for Laravel', () => {
    const env = laravelEnv('https://k@logs.example.com/3', 'staging')
    expect(env).toContain('SENTRY_LARAVEL_DSN=https://k@logs.example.com/3')
    expect(env).toContain('SENTRY_ENVIRONMENT=staging')
    expect(env).toContain('LOG_STACK=daily,sentry_logs')
    const whole = laravelEnv('https://k@logs.example.com/3', null)
    expect(whole).not.toContain('SENTRY_ENVIRONMENT=')
    expect(whole).toContain('# The stage: APP_ENV')
  })
})
