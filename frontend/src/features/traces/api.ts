import { useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

export type Trace = components['schemas']['Trace']
export type TraceTransaction = components['schemas']['TraceTransaction']
export type TraceSpan = components['schemas']['TraceSpan']

export function useTrace(id: MaybeRefOrGetter<string>) {
  return useQuery({
    queryKey: computed(() => ['traces', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/traces/{trace_id}', { params: { path: { trace_id: toValue(id) } } })),
  })
}
