import '@/shared/styles/global.css'

import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from '@/app/App.vue'
import { createAppRouter } from '@/app/router'
import { ApiProblem } from '@/shared/api/client'
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

const app = createApp(App)
app.use(createPinia()).use(i18n).use(VueQueryPlugin, { queryClient }).use(createAppRouter())
app.mount('#app')
