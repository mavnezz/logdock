<script setup lang="ts">
import {
  Activity,
  Bug,
  CalendarClock,
  FolderKanban,
  LogOut,
  type LucideIcon,
  Menu,
  ScrollText,
  ShieldCheck,
  UserRound,
  Users,
} from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'

import { useLogout } from '@/features/auth/api'
import { useSessionStore } from '@/shared/session/store'
import ReadinessBadge from '@/features/system/ReadinessBadge.vue'

import { navigation } from './navigation'

const { t } = useI18n()
const store = useSessionStore()
const route = useRoute()
const router = useRouter()
const logout = useLogout()
const open = ref(false)

const icons: Record<string, LucideIcon> = {
  projects: FolderKanban,
  accounts: Users,
  audit: ShieldCheck,
  profile: UserRound,
  logs: ScrollText,
  issues: Bug,
  performance: Activity,
  monitors: CalendarClock,
}

const items = computed(() => navigation.filter((item) => !item.admin || store.isAdmin))

async function signOut(): Promise<void> {
  await logout.mutateAsync().catch(() => undefined)
  await router.push('/login')
}
</script>

<template>
  <div class="flex min-h-dvh">
    <aside
      class="fixed inset-y-0 left-0 z-40 flex w-60 flex-col border-r bg-card transition-transform md:static md:translate-x-0"
      :class="open ? 'translate-x-0' : '-translate-x-full'"
    >
      <div class="flex h-14 items-center gap-2 border-b px-4">
        <img src="/favicon.svg" alt="" class="size-6" />
        <span class="font-semibold">{{ t('app.name') }}</span>
      </div>
      <nav class="grid gap-0.5 p-2" @click="open = false">
        <template v-for="item in items" :key="item.name">
          <p
            v-if="item.section"
            class="px-3 pt-4 pb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase"
          >
            {{ t(`nav.sections.${item.section}`) }}
          </p>
          <RouterLink
            :to="item.path"
            class="flex items-center gap-2 rounded-md px-3 py-1.5 text-sm hover:bg-accent"
            :class="route.path.startsWith(item.path) && 'bg-accent font-medium'"
          >
            <component :is="icons[item.icon]" class="size-4 text-muted-foreground" />
            {{ t(`nav.${item.name}`) }}
          </RouterLink>
        </template>
      </nav>
      <div class="mt-auto grid gap-3 border-t p-4">
        <ReadinessBadge />
        <div class="flex items-center justify-between gap-2 text-sm">
          <span class="truncate text-muted-foreground">{{ store.account?.email }}</span>
          <button
            type="button"
            class="text-muted-foreground hover:text-foreground"
            :aria-label="t('auth.signOut')"
            @click="signOut"
          >
            <LogOut class="size-4" />
          </button>
        </div>
      </div>
    </aside>
    <div v-if="open" class="fixed inset-0 z-30 bg-black/30 md:hidden" @click="open = false" />
    <div class="flex min-w-0 flex-1 flex-col">
      <header class="flex h-14 items-center gap-2 border-b px-4 md:hidden">
        <button type="button" :aria-label="t('nav.menu')" @click="open = true">
          <Menu class="size-5" />
        </button>
        <span class="font-semibold">{{ t('app.name') }}</span>
      </header>
      <main class="min-w-0 flex-1 p-4 md:p-8">
        <RouterView />
      </main>
    </div>
  </div>
</template>
