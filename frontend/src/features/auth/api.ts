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
