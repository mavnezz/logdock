import { useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

type Schemas = components['schemas']
export type NamedStats = Schemas['NamedStats']
export type SeriesPage = Schemas['SeriesPage']
export type Sample = Schemas['Sample']

export const KINDS = ['request', 'job', 'command', 'query', 'outgoing', 'cache'] as const
export type PerfKind = (typeof KINDS)[number]
export const RANGES = ['1h', '6h', '24h', '7d', '30d'] as const
export type PerfRange = (typeof RANGES)[number]
export const SORTS = ['total_time', 'count', 'slowest', 'failures'] as const
export type PerfSort = (typeof SORTS)[number]

const RANGE_SECONDS: Record<PerfRange, number> = {
  '1h': 3600,
  '6h': 6 * 3600,
  '24h': 86_400,
  '7d': 7 * 86_400,
  '30d': 30 * 86_400,
}

export interface PerfFilters {
  kind: PerfKind
  range: PerfRange
  sort: PerfSort
  project: number | null
  environment: number | null
}

/** The query parameters every performance request takes; the range ends now. */
export function rangeParams(filters: PerfFilters, now: Date): Record<string, string> {
  const params: Record<string, string> = {
    kind: filters.kind,
    from: new Date(now.getTime() - RANGE_SECONDS[filters.range] * 1000).toISOString(),
    to: now.toISOString(),
  }
  if (filters.project !== null) params.project = String(filters.project)
  if (filters.environment !== null) params.environment = String(filters.environment)
  return params
}

export function useOverview(
  params: MaybeRefOrGetter<Record<string, string>>,
  sort: MaybeRefOrGetter<PerfSort>,
) {
  return useQuery({
    queryKey: computed(() => ['performance', toValue(params), toValue(sort)]),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/performance', {
          params: {
            query: {
              ...(toValue(params) as { kind: string }),
              sort: toValue(sort),
              limit: 200,
            },
          },
        }),
      ),
  })
}

export function useSeries(
  params: MaybeRefOrGetter<Record<string, string>>,
  name: MaybeRefOrGetter<string | null>,
) {
  return useQuery({
    queryKey: computed(() => ['performance', 'series', toValue(params), toValue(name)]),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/performance/series', {
          params: {
            query: { ...(toValue(params) as { kind: string }), name: toValue(name) ?? '' },
          },
        }),
      ),
    enabled: computed(() => toValue(name) !== null),
  })
}

export function useSamples(
  params: MaybeRefOrGetter<Record<string, string>>,
  name: MaybeRefOrGetter<string | null>,
  enabled: MaybeRefOrGetter<boolean>,
) {
  return useQuery({
    queryKey: computed(() => ['performance', 'samples', toValue(params), toValue(name)]),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/performance/samples', {
          params: {
            query: {
              ...(toValue(params) as { kind: string }),
              name: toValue(name) ?? '',
              limit: 10,
            },
          },
        }),
      ),
    enabled: computed(() => toValue(name) !== null && toValue(enabled)),
  })
}

export type QueueStats = Schemas['QueueStats']

/** The queues over a range: put in, ran, failed, waited and ran for. */
export function useQueues(params: MaybeRefOrGetter<Record<string, string>>) {
  return useQuery({
    queryKey: computed(() => ['performance', 'queues', toValue(params)]),
    queryFn: () => {
      const { from, to, project, environment } = toValue(params)
      return unwrap(
        api.GET('/api/v1/queues', { params: { query: { from, to, project, environment } } }),
      )
    },
  })
}
