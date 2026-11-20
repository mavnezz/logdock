import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'

import { api, unwrap } from '@/shared/api/client'
import type { AccountRole } from '@/shared/api/types'

export function useAccounts() {
  return useQuery({ queryKey: ['accounts'], queryFn: () => unwrap(api.GET('/api/v1/accounts')) })
}

const AUDIT_PAGE = 100

/** The audit log, newest first, a page at a time. */
export function useAudit() {
  return useInfiniteQuery({
    queryKey: ['audit'],
    queryFn: ({ pageParam }) =>
      unwrap(
        api.GET('/api/v1/audit', {
          params: { query: { limit: AUDIT_PAGE, before: pageParam ?? undefined } },
        }),
      ),
    initialPageParam: null as number | null,
    getNextPageParam: (page) =>
      page.length === AUDIT_PAGE ? (page[page.length - 1]?.id ?? undefined) : undefined,
  })
}

function useAccountMutation<T, R>(mutationFn: (input: T) => Promise<R>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['accounts'] }),
  })
}

export function useCreateAccount() {
  return useAccountMutation(
    (body: { email: string; name: string; role: AccountRole; password: string; locale: string }) =>
      unwrap(api.POST('/api/v1/accounts', { body })),
  )
}

export function useUpdateAccount() {
  return useAccountMutation(
    ({ id, ...body }: { id: number; name?: string; role?: AccountRole; disabled?: boolean }) =>
      unwrap(api.PATCH('/api/v1/accounts/{id}', { params: { path: { id } }, body })),
  )
}

export function useResetPassword() {
  return useAccountMutation(({ id, password }: { id: number; password: string }) =>
    unwrap(
      api.POST('/api/v1/accounts/{id}/password', { params: { path: { id } }, body: { password } }),
    ),
  )
}

export function useDeleteAccount() {
  return useAccountMutation((id: number) =>
    unwrap(api.DELETE('/api/v1/accounts/{id}', { params: { path: { id } } })),
  )
}
