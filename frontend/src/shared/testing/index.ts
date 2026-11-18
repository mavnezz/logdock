// Helpers for component tests: mounting with the app's plugins, and a
// stand-in for the API that answers by path.

import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { mount, type ComponentMountingOptions } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { vi } from 'vitest'
import type { Component } from 'vue'
import { createMemoryHistory, createRouter, type RouteRecordRaw } from 'vue-router'

import { i18n, setLocale } from '@/shared/i18n'

type Handler = (request: Request) => unknown

/** An answer the stand-in API gives, with its status. */
export function reply(status: number, body: unknown): Response {
  return new Response(body === undefined ? null : JSON.stringify(body), {
    status,
    headers: { 'Content-Type': status >= 400 ? 'application/problem+json' : 'application/json' },
  })
}

/**
 * Answers `fetch` by `METHOD /path` (without the query string); a handler
 * returns a body for 200 or a {@link Response}. Unknown paths answer 404.
 */
export function stubApi(routes: Record<string, Handler>) {
  const calls: Request[] = []
  const fetch = vi.fn((input: Request) => {
    calls.push(input)
    const url = new URL(input.url)
    const handler = routes[`${input.method} ${url.pathname}`]
    if (!handler) return Promise.resolve(reply(404, { code: 'not_found' }))
    const result = handler(input)
    return Promise.resolve(result instanceof Response ? result : reply(200, result))
  })
  vi.stubGlobal('fetch', fetch)
  return { fetch, calls }
}

export function mountWith<T extends Component>(
  component: T,
  options: ComponentMountingOptions<T> & { routes?: RouteRecordRaw[]; path?: string } = {},
) {
  setLocale('en')
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const routes = options.routes ?? [{ path: '/:any(.*)*', component: { template: '<div />' } }]
  const router = createRouter({ history: createMemoryHistory(), routes })
  void router.push(options.path ?? '/')
  const wrapper = mount(component, {
    ...options,
    global: {
      plugins: [createPinia(), i18n, [VueQueryPlugin, { queryClient }], router],
      ...options.global,
    },
  })
  return { wrapper, router, queryClient }
}
