import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'

import { api, unwrap } from '@/shared/api/client'

import { useSessionStore } from '@/shared/session/store'

export function useSetupStatus() {
  return useQuery({
    queryKey: ['setup'],
    queryFn: () => unwrap(api.GET('/api/v1/setup')),
  })
}

export interface SetupForm {
  code: string
  email: string
  name: string
  password: string
  locale: string
}

export function useSetup() {
  const store = useSessionStore()
  return useMutation({
    mutationFn: (body: SetupForm) => unwrap(api.POST('/api/v1/setup', { body })),
    onSuccess: (session) => store.set(session),
  })
}

export function useLogin() {
  const store = useSessionStore()
  return useMutation({
    mutationFn: (body: { email: string; password: string }) =>
      unwrap(api.POST('/api/v1/auth/login', { body })),
    onSuccess: (session) => store.set(session),
  })
}

export function useLogout() {
  const store = useSessionStore()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/auth/logout')),
    onSettled: () => {
      store.clear()
      queryClient.clear()
    },
  })
}

export function useUpdateProfile() {
  const store = useSessionStore()
  return useMutation({
    mutationFn: (body: { name?: string; locale?: string }) =>
      unwrap(api.PATCH('/api/v1/me', { body })),
    onSuccess: (account) => {
      if (store.session) store.set({ ...store.session, account })
    },
  })
}

export function useChangePassword() {
  return useMutation({
    mutationFn: (body: { current: string; new: string }) =>
      unwrap(api.POST('/api/v1/me/password', { body })),
  })
}

export function useTokens() {
  return useQuery({
    queryKey: ['tokens'],
    queryFn: () => unwrap(api.GET('/api/v1/me/tokens')),
  })
}

export function useCreateToken() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: { name: string; scope: 'read' | 'triage' }) =>
      unwrap(api.POST('/api/v1/me/tokens', { body })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tokens'] }),
  })
}

export function useRevokeToken() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) =>
      unwrap(api.DELETE('/api/v1/me/tokens/{id}', { params: { path: { id } } })),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tokens'] }),
  })
}
