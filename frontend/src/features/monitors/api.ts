import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Monitor = components['schemas']['Monitor']
export type MonitorDetail = components['schemas']['MonitorDetail']
export type CheckIn = components['schemas']['CheckInDto']

export function useMonitors(
  project: MaybeRefOrGetter<number | null>,
  environment: MaybeRefOrGetter<number | null>,
) {
  return useQuery({
    queryKey: computed(() => ['monitors', toValue(project), toValue(environment)]),
    queryFn: () => {
      const query: Record<string, string> = {}
      const p = toValue(project)
      const e = toValue(environment)
      if (p !== null) query.project = String(p)
      if (e !== null) query.environment = String(e)
      return unwrap(api.GET('/api/v1/monitors', { params: { query } }))
    },
    refetchInterval: 30_000,
  })
}

export function useMonitor(id: MaybeRefOrGetter<number>) {
  return useQuery({
    queryKey: computed(() => ['monitors', 'detail', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/monitors/{id}', { params: { path: { id: toValue(id) } } })),
    refetchInterval: 30_000,
  })
}

export function useDeleteMonitor(id: MaybeRefOrGetter<number>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () =>
      unwrap(api.DELETE('/api/v1/monitors/{id}', { params: { path: { id: toValue(id) } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['monitors'] }),
  })
}

/** How a health or run status looks: a badge variant. */
export function variant(
  status: string | null | undefined,
): 'success' | 'destructive' | 'warning' | 'secondary' {
  if (status === 'ok') return 'success'
  if (status === 'failing' || status === 'error' || status === 'timeout') return 'destructive'
  if (status === 'missed') return 'warning'
  return 'secondary'
}
