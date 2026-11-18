import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { api, ApiProblem, unwrap } from '@/shared/api/client'
import type { Session } from '@/shared/api/types'
import { setLocale } from '@/shared/i18n'

/** The signed-in account and its session, once asked for. */
export const useSessionStore = defineStore('session', () => {
  const session = ref<Session | null>(null)
  const checked = ref(false)

  const account = computed(() => session.value?.account ?? null)
  const isAdmin = computed(() => account.value?.role === 'admin')

  function set(next: Session): void {
    session.value = next
    checked.value = true
    const locale = next.account.locale
    if (locale === 'de' || locale === 'en') setLocale(locale)
  }

  function clear(): void {
    session.value = null
    checked.value = true
  }

  /** Asks the server once whether there is a session. */
  async function load(): Promise<Session | null> {
    if (checked.value) return session.value
    try {
      set(await unwrap(api.GET('/api/v1/auth/session')))
    } catch (error) {
      if (!(error instanceof ApiProblem) || error.status !== 401) throw error
      clear()
    }
    return session.value
  }

  return { session, checked, account, isAdmin, set, clear, load }
})
