import { keepPreviousData, useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Overview = components['schemas']['Overview']

export const RANGES = ['1h', '6h', '24h', '7d', '30d'] as const
export type OverviewRange = (typeof RANGES)[number]

const RANGE_SECONDS: Record<OverviewRange, number> = {
  '1h': 3600,
  '6h': 6 * 3600,
  '24h': 86_400,
  '7d': 7 * 86_400,
  '30d': 30 * 86_400,
}

export interface OverviewFilters {
  range: OverviewRange
  project: number | null
  environment: number | null
}

/** The query of an overview of `range` up to `now`. */
export function overviewParams(filters: OverviewFilters, now: Date): Record<string, string> {
  const params: Record<string, string> = {
    from: new Date(now.getTime() - RANGE_SECONDS[filters.range] * 1000).toISOString(),
    to: now.toISOString(),
  }
  if (filters.project !== null) params.project = String(filters.project)
  if (filters.environment !== null) params.environment = String(filters.environment)
  return params
}

export function useOverview(params: MaybeRefOrGetter<Record<string, string>>) {
  return useQuery({
    queryKey: computed(() => ['overview', toValue(params)]),
    queryFn: () => unwrap(api.GET('/api/v1/overview', { params: { query: toValue(params) } })),
    // The range moves on while the page is open: show the last numbers
    // until the new ones are there.
    placeholderData: keepPreviousData,
  })
}
