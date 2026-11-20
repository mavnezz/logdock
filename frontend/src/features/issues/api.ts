import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { api, unwrap } from '@/shared/api/client'
import type { components } from '@/shared/api/schema'

type Schemas = components['schemas']
export type Issue = Schemas['Issue']
export type IssueListItem = Schemas['IssueListItem']
export type IssueDetail = Schemas['IssueDetailDto']
export type ErrorEvent = Schemas['Event']
export type EventListItem = Schemas['EventListItem']
export type StackFrame = Schemas['StackFrame']
export type ChangeIssue = Schemas['ChangeIssueRequest']

export const STATUSES = ['unresolved', 'resolved', 'muted', 'all'] as const
export type StatusFilter = (typeof STATUSES)[number]
export const SORTS = ['last_seen', 'first_seen', 'count'] as const
export type Sort = (typeof SORTS)[number]

export interface IssueFilters {
  project: number | null
  environment: number | null
  status: StatusFilter
  sort: Sort
  q: string
  /** Only issues that first appeared in this release. */
  release: string
  /** From 1. */
  page: number
}

export const PAGE_SIZE = 50

/** One page of issues, and whether another follows. */
export function useIssues(filters: MaybeRefOrGetter<IssueFilters>) {
  return useQuery({
    queryKey: computed(() => ['issues', toValue(filters)]),
    queryFn: async () => {
      const f = toValue(filters)
      const query: Record<string, string> = {
        status: f.status,
        sort: f.sort,
        // One more than shown tells whether a next page exists.
        limit: String(PAGE_SIZE + 1),
        offset: String((f.page - 1) * PAGE_SIZE),
      }
      if (f.project !== null) query.project = String(f.project)
      if (f.environment !== null) query.environment = String(f.environment)
      if (f.q.trim()) query.q = f.q.trim()
      if (f.release) query.release = f.release
      const items = await unwrap(api.GET('/api/v1/issues', { params: { query } }))
      return { items: items.slice(0, PAGE_SIZE), hasNext: items.length > PAGE_SIZE }
    },
    placeholderData: (previous) => previous,
    refetchInterval: 30_000,
  })
}

export function useIssue(id: MaybeRefOrGetter<number>) {
  return useQuery({
    queryKey: computed(() => ['issues', 'detail', toValue(id)]),
    queryFn: () =>
      unwrap(api.GET('/api/v1/issues/{id}', { params: { path: { id: toValue(id) } } })),
    refetchInterval: 30_000,
  })
}

const EVENTS_PAGE = 50

/** An issue's events, newest first, a page at a time. */
export function useIssueEvents(id: MaybeRefOrGetter<number>) {
  return useInfiniteQuery({
    queryKey: computed(() => ['issues', 'detail', toValue(id), 'events']),
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET('/api/v1/issues/{id}/events', {
          params: {
            path: { id: toValue(id) },
            query: { limit: EVENTS_PAGE, before: pageParam || undefined },
          },
        }),
      ),
    initialPageParam: '',
    getNextPageParam: (page) =>
      page.length === EVENTS_PAGE ? (page[page.length - 1]?.time ?? undefined) : undefined,
  })
}

/** One event by its ID; none while `id` is `null` (the issue's latest is shown then). */
export function useEvent(id: MaybeRefOrGetter<string | null>) {
  return useQuery({
    queryKey: computed(() => ['events', toValue(id)]),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/events/{event_id}', {
          params: { path: { event_id: toValue(id) ?? '' } },
        }),
      ),
    enabled: computed(() => toValue(id) !== null),
  })
}

export function useTrace(id: MaybeRefOrGetter<string | null>) {
  return useQuery({
    queryKey: computed(() => ['traces', toValue(id)]),
    queryFn: () =>
      unwrap(
        api.GET('/api/v1/traces/{trace_id}', { params: { path: { trace_id: toValue(id) ?? '' } } }),
      ),
    enabled: computed(() => toValue(id) !== null),
  })
}

/** Where an issue's recent events came from: release, route, service, host. */
export function useIssueTags(id: MaybeRefOrGetter<number>) {
  return useQuery({
    queryKey: computed(() => ['issues', 'detail', toValue(id), 'tags']),
    queryFn: () =>
      unwrap(api.GET('/api/v1/issues/{id}/tags', { params: { path: { id: toValue(id) } } })),
  })
}

/** Resolving, muting and reopening refresh every issue list and page. */
export function useChangeIssue(id: MaybeRefOrGetter<number>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: ChangeIssue) =>
      unwrap(api.PATCH('/api/v1/issues/{id}', { params: { path: { id: toValue(id) } }, body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['issues'] }),
  })
}

export function useDeleteIssue(id: MaybeRefOrGetter<number>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () =>
      unwrap(api.DELETE('/api/v1/issues/{id}', { params: { path: { id: toValue(id) } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['issues'] }),
  })
}

/** The ways to mute an issue the page offers. */
export const MUTES: { key: string; change: ChangeIssue }[] = [
  { key: 'hour', change: { action: 'mute', mute_minutes: 60 } },
  { key: 'day', change: { action: 'mute', mute_minutes: 24 * 60 } },
  { key: 'week', change: { action: 'mute', mute_minutes: 7 * 24 * 60 } },
  { key: 'events10', change: { action: 'mute', mute_events: 10 } },
  { key: 'events100', change: { action: 'mute', mute_events: 100 } },
  { key: 'forever', change: { action: 'mute' } },
]
