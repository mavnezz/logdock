import { useQuery } from '@tanstack/vue-query'

import { api, ApiProblem } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Health = components['schemas']['Health']

/** Whether Logdock can take data, refreshed every half minute. */
export function useReadiness() {
  return useQuery({
    queryKey: ['system', 'ready'],
    queryFn: async (): Promise<Health> => {
      const { data, error, response } = await api.GET('/health/ready')
      // 503 carries the health body too, with its problems.
      const health = data ?? error
      if (health?.status) return health
      throw new ApiProblem(response.status, 'unknown')
    },
    refetchInterval: 30_000,
  })
}
