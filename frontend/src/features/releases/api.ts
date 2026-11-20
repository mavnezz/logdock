import { useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Release = components['schemas']['Release']

/** Releases, newest first, of the chosen project and stage. */
export function useReleases(
  scope: MaybeRefOrGetter<{ project: number | null; environment: number | null }>,
) {
  return useQuery({
    queryKey: computed(() => ['releases', toValue(scope)]),
    queryFn: () => {
      const { project, environment } = toValue(scope)
      const query: Record<string, string> = {}
      if (project !== null) query.project = String(project)
      if (environment !== null) query.environment = String(environment)
      return unwrap(api.GET('/api/v1/releases', { params: { query } }))
    },
    refetchInterval: 60_000,
  })
}

/** A request failure rate in percent; `null` without requests. */
export function failureRate(release: Release): number | null {
  const requests = release.requests
  return requests && requests.count > 0 ? (requests.flagged / requests.count) * 100 : null
}
