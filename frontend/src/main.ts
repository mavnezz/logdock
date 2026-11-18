import '@/shared/styles/global.css'

import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from '@/app/App.vue'
import { createAppRouter } from '@/app/router'
import { useSessionStore } from '@/shared/session/store'
import { ApiProblem, configureApi } from '@/shared/api/client'
import { i18n } from '@/shared/i18n'
import { applyTheme, followSystemTheme } from '@/shared/lib/theme'

applyTheme()
followSystemTheme()

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Client errors do not go away by asking again.
      retry: (failures, error) =>
        !(error instanceof ApiProblem && error.status >= 400 && error.status < 500) && failures < 2,
    },
  },
})

const pinia = createPinia()
const router = createAppRouter(pinia)
const session = useSessionStore(pinia)

configureApi({
  csrfToken: () => session.session?.csrf_token,
  onUnauthenticated: () => {
    session.clear()
    queryClient.clear()
    void router.push({ name: 'login' })
  },
})

createApp(App).use(pinia).use(i18n).use(VueQueryPlugin, { queryClient }).use(router).mount('#app')
