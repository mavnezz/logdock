import { useInfiniteQuery, useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type LogEntry = components['schemas']['LogEntry']
export type Histogram = components['schemas']['Histogram']

type Params = Record<string, string>

export function useLogPages(params: MaybeRefOrGetter<Params>) {
  return useInfiniteQuery({
    queryKey: computed(() => ['logs', toValue(params)]),
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET('/api/v1/logs', {
          params: { query: { ...toValue(params), limit: 100, before: pageParam || undefined } },
        }),
      ),
    initialPageParam: '',
    getNextPageParam: (page) => page.next ?? undefined,
  })
}

export function useHistogram(params: MaybeRefOrGetter<Params>) {
  return useQuery({
    queryKey: computed(() => ['logs', 'histogram', toValue(params)]),
    queryFn: () => {
      // The counts know no text: only the filters they are kept by.
      const query = { ...toValue(params) }
      delete query.q
      delete query.host
      delete query.trace
      return unwrap(api.GET('/api/v1/logs/histogram', { params: { query } }))
    },
  })
}

export function useSources(project: MaybeRefOrGetter<number | null>) {
  return useQuery({
    queryKey: computed(() => ['logs', 'sources', toValue(project)]),
    queryFn: () => {
      const id = toValue(project)
      return unwrap(
        api.GET('/api/v1/logs/sources', {
          params: { query: id === null ? {} : { project: String(id) } },
        }),
      )
    },
    staleTime: 60_000,
  })
}

export function useLogEntry(id: MaybeRefOrGetter<string | null>) {
  return useQuery({
    queryKey: computed(() => ['logs', 'entry', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/logs/{id}', { params: { path: { id: toValue(id) ?? '' } } })),
    enabled: computed(() => toValue(id) !== null),
  })
}

/** New entries matching `params` as they are written; returns a function that stops listening. */
export function openStream(
  params: Params,
  onEntry: (entry: LogEntry) => void,
  onLagged: (missed: number) => void,
): () => void {
  // The live view shows what comes from now on.
  const query = new URLSearchParams(params)
  query.delete('from')
  query.delete('to')
  const source = new EventSource(`/api/v1/logs/stream?${query.toString()}`)
  source.addEventListener('entry', (event) =>
    onEntry(JSON.parse((event as MessageEvent<string>).data) as LogEntry),
  )
  source.addEventListener('lagged', (event) =>
    onLagged(Number((event as MessageEvent<string>).data)),
  )
  return () => source.close()
}
