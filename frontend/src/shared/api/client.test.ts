import { describe, expect, it, vi } from 'vitest'

import { ApiProblem, configureApi, createApiClient, unwrap } from './client'

function answer(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  })
}

describe('the API client', () => {
  it('returns the data of a successful answer', async () => {
    const fetch = vi.fn(() => Promise.resolve(answer(200, { status: 'live' })))
    const client = createApiClient({ baseUrl: 'http://logdock.test', fetch })
    await expect(unwrap(client.GET('/health/live'))).resolves.toEqual({ status: 'live' })
  })

  it('turns error answers into problems with their code and Retry-After', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(answer(503, { code: 'low_disk', detail: 'full' }, { 'Retry-After': '30' })),
    )
    const client = createApiClient({ baseUrl: 'http://logdock.test', fetch })
    const problem = await unwrap(client.GET('/health/ready')).catch((e: unknown) => e)
    expect(problem).toBeInstanceOf(ApiProblem)
    expect(problem).toMatchObject({
      status: 503,
      code: 'low_disk',
      detail: 'full',
      retryAfterSeconds: 30,
    })
  })

  it('reports a network failure as such', async () => {
    const fetch = vi.fn(() => Promise.reject(new TypeError('offline')))
    const client = createApiClient({ baseUrl: 'http://logdock.test', fetch })
    await expect(unwrap(client.GET('/health/live'))).rejects.toMatchObject({ code: 'network' })
  })

  it('sends the CSRF token with changes and reports an ended session', async () => {
    const onUnauthenticated = vi.fn()
    configureApi({ csrfToken: () => 'token-1', onUnauthenticated })
    const seen: Request[] = []
    const fetch = vi.fn((request: Request) => {
      seen.push(request)
      return Promise.resolve(answer(401, { code: 'unauthenticated' }))
    })
    const client = createApiClient({
      baseUrl: 'http://logdock.test',
      fetch: fetch as unknown as typeof globalThis.fetch,
    })
    await client.GET('/health/live').catch(() => undefined)
    expect(seen[0]?.headers.get('X-CSRF-Token')).toBeNull()
    expect(onUnauthenticated).toHaveBeenCalledTimes(1)
    configureApi({ csrfToken: () => undefined, onUnauthenticated: () => undefined })
  })
})
