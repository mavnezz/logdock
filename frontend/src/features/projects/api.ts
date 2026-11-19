import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { ProjectSettings } from '@/shared/api/types'

export function useProjects() {
  return useQuery({
    queryKey: ['projects'],
    queryFn: () => unwrap(api.GET('/api/v1/projects')),
  })
}

export function useProject(id: MaybeRefOrGetter<number>) {
  return useQuery({
    queryKey: computed(() => ['projects', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/projects/{id}', { params: { path: { id: toValue(id) } } })),
  })
}

export function useKeys(id: MaybeRefOrGetter<number>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery({
    queryKey: computed(() => ['projects', toValue(id), 'keys']),
    queryFn: () =>
      unwrap(api.GET('/api/v1/projects/{id}/keys', { params: { path: { id: toValue(id) } } })),
    enabled: computed(() => toValue(enabled)),
  })
}

export function useIngestCounts(id: MaybeRefOrGetter<number>) {
  return useQuery({
    queryKey: computed(() => ['projects', toValue(id), 'ingest']),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/projects/{id}/ingest', {
          params: { path: { id: toValue(id) }, query: { days: 7 } },
        }),
      ),
    refetchInterval: 30_000,
  })
}

/** Every change to a project refreshes what is shown of projects. */
function useProjectMutation<T, R>(mutationFn: (input: T) => Promise<R>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['projects'] }),
  })
}

export function useCreateProject() {
  return useProjectMutation((body: { name: string; environments: string[] }) =>
    unwrap(api.POST('/api/v1/projects', { body })),
  )
}

export function useTestMail(id: MaybeRefOrGetter<number>) {
  return useMutation({
    mutationFn: () =>
      unwrap(
        api.POST('/api/v1/projects/{id}/test-mail', { params: { path: { id: toValue(id) } } }),
      ),
  })
}

export function useUpdateProject(id: MaybeRefOrGetter<number>) {
  return useProjectMutation((body: { name?: string; settings?: ProjectSettings }) =>
    unwrap(api.PATCH('/api/v1/projects/{id}', { params: { path: { id: toValue(id) } }, body })),
  )
}

export function useDeleteProject() {
  return useProjectMutation((id: number) =>
    unwrap(api.DELETE('/api/v1/projects/{id}', { params: { path: { id } } })),
  )
}

export function useAddEnvironment(id: MaybeRefOrGetter<number>) {
  return useProjectMutation((name: string) =>
    unwrap(
      api.POST('/api/v1/projects/{id}/environments', {
        params: { path: { id: toValue(id) } },
        body: { name },
      }),
    ),
  )
}

export function useRemoveEnvironment(id: MaybeRefOrGetter<number>) {
  return useProjectMutation((environment: number) =>
    unwrap(
      api.DELETE('/api/v1/projects/{id}/environments/{environment}', {
        params: { path: { id: toValue(id), environment } },
      }),
    ),
  )
}

export function useCreateKey(id: MaybeRefOrGetter<number>) {
  return useProjectMutation((body: { environment_id: number; label: string }) =>
    unwrap(api.POST('/api/v1/projects/{id}/keys', { params: { path: { id: toValue(id) } }, body })),
  )
}

export function useRevokeKey(id: MaybeRefOrGetter<number>) {
  return useProjectMutation((key: number) =>
    unwrap(
      api.DELETE('/api/v1/projects/{id}/keys/{key}', {
        params: { path: { id: toValue(id), key } },
      }),
    ),
  )
}
