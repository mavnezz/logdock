<script setup lang="ts">
import {
  ChevronRight,
  LogOut,
  Menu,
  Monitor,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Sun,
  X,
} from 'lucide-vue-next'
import { computed, ref, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'

import { useLogout } from '@/features/auth/api'
import ReadinessBadge from '@/features/system/ReadinessBadge.vue'
import { setThemePreference, themeChoice, type ThemePreference } from '@/shared/lib/theme'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useSessionStore } from '@/shared/session/store'
import { Kbd } from '@/shared/ui/kbd'

import CommandPalette from './CommandPalette.vue'
import { NAV_ICONS } from './icons'
import { navigation, navItemOf } from './navigation'
import { useShortcuts } from './shortcuts'

const { t } = useI18n()
const store = useSessionStore()
const route = useRoute()
const router = useRouter()
const logout = useLogout()
/** The sidebar on a phone. */
const open = ref(false)
const palette = ref(false)
const main = useTemplateRef<HTMLElement>('main')

const COLLAPSED_KEY = 'logdock.sidebar'
function storedCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === 'collapsed'
  } catch {
    return false
  }
}
/** The sidebar as icons only, on a wide screen. */
const collapsed = ref(storedCollapsed())
watch(collapsed, (now) => {
  try {
    if (now) localStorage.setItem(COLLAPSED_KEY, 'collapsed')
    else localStorage.removeItem(COLLAPSED_KEY)
  } catch {
    // Storage unavailable: for this page only.
  }
})

const media =
  typeof window.matchMedia === 'function' ? window.matchMedia('(min-width: 768px)') : null
const wide = ref(media?.matches ?? true)
media?.addEventListener('change', (event) => (wide.value = event.matches))
/** Icons only: collapsed, where there is room for the sidebar at all. */
const rail = computed(() => collapsed.value && wide.value)

const items = computed(() => navigation.filter((item) => !item.admin || store.isAdmin))
const current = computed(() => navItemOf(route.path))
const section = computed(() => {
  const at = current.value ? navigation.indexOf(current.value) : -1
  return navigation
    .slice(0, at + 1)
    .reverse()
    .find((item) => item.section)?.section
})
const detail = computed(() => {
  const id = route.params.id
  if (typeof id !== 'string') return null
  return route.path.startsWith('/issues/') ? `#${id}` : id.length > 12 ? id.slice(0, 8) : id
})
const isMac = typeof navigator !== 'undefined' && /mac|iphone|ipad/i.test(navigator.platform)

const THEMES: Record<ThemePreference, { next: ThemePreference; icon: typeof Sun }> = {
  system: { next: 'light', icon: Monitor },
  light: { next: 'dark', icon: Sun },
  dark: { next: 'system', icon: Moon },
}
const theme = computed(() => THEMES[themeChoice.value])
const initials = computed(() => (store.account?.email ?? '?').slice(0, 1).toUpperCase())

// A new page starts at its top; the panel, not the window, scrolls.
watch(
  () => route.path,
  () => {
    open.value = false
    main.value?.scrollTo?.({ top: 0 })
  },
)

useShortcuts({
  palette: () => (palette.value = true),
  go: (path) => void router.push(path),
  allowed: (path) => items.value.some((i) => i.path === path),
})

async function signOut(): Promise<void> {
  await logout.mutateAsync().catch(() => undefined)
  await router.push('/login')
}
</script>

<template>
  <div class="flex h-dvh overflow-hidden bg-canvas">
    <aside
      class="fixed inset-y-0 left-0 z-40 flex w-64 flex-col bg-canvas transition-[transform,width] duration-200 md:static md:translate-x-0"
      :class="[
        open ? 'translate-x-0 shadow-pop' : '-translate-x-full',
        rail ? 'md:w-[3.75rem]' : 'md:w-60',
      ]"
      :aria-label="t('nav.menu')"
    >
      <div
        class="flex h-14 shrink-0 items-center gap-2.5 px-3.5"
        :class="rail && 'justify-center px-0'"
      >
        <RouterLink
          to="/overview"
          class="flex min-w-0 items-center gap-2.5"
          :aria-label="t('app.name')"
        >
          <img src="/favicon.svg" alt="" class="size-7 shrink-0 rounded-lg shadow-xs" />
          <span
            class="truncate text-[15px] font-semibold tracking-tight"
            :class="rail && 'hidden'"
            >{{ t('app.name') }}</span
          >
        </RouterLink>
        <button
          type="button"
          class="ml-auto hidden size-7 cursor-pointer place-items-center rounded-md text-muted-foreground hover:bg-foreground/5 hover:text-foreground md:grid"
          :class="rail && 'md:hidden'"
          :aria-label="t('nav.collapse')"
          :title="t('nav.collapse')"
          @click="collapsed = true"
        >
          <PanelLeftClose class="size-4" />
        </button>
        <button
          type="button"
          class="ml-auto grid size-7 cursor-pointer place-items-center rounded-md text-muted-foreground hover:bg-foreground/5 md:hidden"
          :aria-label="t('common.close')"
          @click="open = false"
        >
          <X class="size-4" />
        </button>
      </div>

      <div class="px-2.5 pb-2" :class="rail && 'px-2'">
        <button
          type="button"
          class="flex h-8 w-full cursor-pointer items-center gap-2 rounded-lg border bg-background px-2.5 text-sm text-muted-foreground shadow-xs transition-colors hover:border-foreground/20 hover:text-foreground"
          :class="rail && 'justify-center px-0'"
          :aria-label="t('nav.search')"
          :title="rail ? t('nav.search') : undefined"
          @click="palette = true"
        >
          <Search class="size-3.5 shrink-0" />
          <span class="truncate" :class="rail && 'hidden'">{{ t('nav.searchShort') }}</span>
          <span class="ml-auto flex gap-0.5" :class="rail && 'hidden'">
            <Kbd>{{ isMac ? '⌘' : 'Ctrl' }}</Kbd><Kbd>K</Kbd>
          </span>
        </button>
      </div>

      <nav
        class="grid min-h-0 flex-1 content-start gap-px overflow-y-auto px-2.5 pb-3"
        :class="rail && 'px-2'"
      >
        <template v-for="item in items" :key="item.name">
          <p
            v-if="item.section"
            class="px-2.5 pt-4 pb-1.5 text-[11px] font-medium text-muted-foreground/80"
            :class="rail && 'hidden'"
          >
            {{ t(`nav.sections.${item.section}`) }}
          </p>
          <hr v-if="item.section && rail" class="mx-2 my-2.5" />
          <RouterLink
            :to="item.path"
            class="group flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-foreground/5 hover:text-foreground"
            :class="[
              current?.name === item.name &&
                'bg-background font-medium !text-foreground shadow-panel hover:bg-background',
              rail && 'justify-center px-0',
            ]"
            :title="rail ? t(`nav.${item.name}`) : undefined"
            :aria-current="current?.name === item.name ? 'page' : undefined"
          >
            <component
              :is="NAV_ICONS[item.icon]"
              class="size-4 shrink-0"
              :class="
                current?.name === item.name
                  ? 'text-primary'
                  : 'text-muted-foreground group-hover:text-foreground'
              "
            />
            <span class="truncate" :class="rail && 'hidden'">{{ t(`nav.${item.name}`) }}</span>
          </RouterLink>
        </template>
      </nav>

      <div class="grid shrink-0 gap-2 border-t border-foreground/5 p-2.5" :class="rail && 'px-2'">
        <div class="px-1.5 py-1">
          <ReadinessBadge :compact="rail" />
        </div>
        <div class="flex items-center gap-1" :class="rail && 'flex-col'">
          <RouterLink
            to="/profile"
            class="flex min-w-0 flex-1 items-center gap-2 rounded-lg p-1.5 hover:bg-foreground/5"
            :class="rail && 'flex-none'"
            :title="store.account?.email"
          >
            <span
              class="grid size-6 shrink-0 place-items-center rounded-full bg-gradient-to-br from-primary/80 to-primary text-[11px] font-semibold text-primary-foreground"
              aria-hidden="true"
              >{{ initials }}</span
            >
            <span class="truncate text-xs" :class="rail && 'hidden'">{{
              store.account?.email
            }}</span>
          </RouterLink>
          <button
            type="button"
            class="grid size-7 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground hover:bg-foreground/5 hover:text-foreground"
            :aria-label="t('nav.theme', { theme: t(`profile.themes.${themeChoice}`) })"
            :title="t('nav.theme', { theme: t(`profile.themes.${themeChoice}`) })"
            @click="setThemePreference(theme.next)"
          >
            <component :is="theme.icon" class="size-4" />
          </button>
          <button
            type="button"
            class="grid size-7 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground hover:bg-foreground/5 hover:text-foreground"
            :aria-label="t('auth.signOut')"
            :title="t('auth.signOut')"
            @click="signOut"
          >
            <LogOut class="size-4" />
          </button>
        </div>
        <button
          v-if="rail"
          type="button"
          class="grid size-7 cursor-pointer place-items-center justify-self-center rounded-md text-muted-foreground hover:bg-foreground/5 hover:text-foreground"
          :aria-label="t('nav.expand')"
          :title="t('nav.expand')"
          @click="collapsed = false"
        >
          <PanelLeftOpen class="size-4" />
        </button>
      </div>
    </aside>
    <div
      v-if="open"
      class="fixed inset-0 z-30 animate-fade-in bg-black/30 backdrop-blur-[2px] md:hidden"
      @click="open = false"
    />

    <div class="flex min-w-0 flex-1 flex-col md:py-2 md:pr-2">
      <div
        class="flex min-h-0 flex-1 flex-col overflow-hidden bg-background md:rounded-xl md:shadow-panel"
      >
        <header class="flex h-12 shrink-0 items-center gap-2 border-b px-3 md:px-5">
          <button
            type="button"
            class="grid size-8 cursor-pointer place-items-center rounded-md hover:bg-accent md:hidden"
            :aria-label="t('nav.menu')"
            @click="open = true"
          >
            <Menu class="size-5" />
          </button>
          <nav
            :aria-label="t('nav.breadcrumb')"
            class="min-w-0"
            :class="route.meta.scoped && 'max-sm:hidden'"
          >
            <ol class="flex items-center gap-1.5 text-sm">
              <template v-if="current">
                <template v-if="section">
                  <li class="text-muted-foreground max-md:hidden">
                    {{ t(`nav.sections.${section}`) }}
                  </li>
                  <li class="text-muted-foreground/50 max-md:hidden" aria-hidden="true">
                    <ChevronRight class="size-3.5" />
                  </li>
                </template>
                <li class="truncate">
                  <RouterLink
                    v-if="detail"
                    :to="current.path"
                    class="text-muted-foreground hover:text-foreground"
                    >{{ t(`nav.${current.name}`) }}</RouterLink
                  >
                  <span v-else class="font-medium" aria-current="page">{{
                    t(`nav.${current.name}`)
                  }}</span>
                </li>
              </template>
              <li v-else-if="route.path.startsWith('/traces/')" class="text-muted-foreground">
                {{ t('traces.title') }}
              </li>
              <template v-if="detail">
                <li class="text-muted-foreground/50" aria-hidden="true">
                  <ChevronRight class="size-3.5" />
                </li>
                <li class="truncate font-mono text-[13px] font-medium" aria-current="page">
                  {{ detail }}
                </li>
              </template>
            </ol>
          </nav>
          <div class="ml-auto flex items-center gap-2">
            <ScopePicker v-if="route.meta.scoped" />
            <button
              type="button"
              class="grid size-8 cursor-pointer place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground md:hidden"
              :aria-label="t('nav.search')"
              @click="palette = true"
            >
              <Search class="size-4" />
            </button>
          </div>
        </header>
        <main ref="main" class="min-h-0 flex-1 overflow-y-auto">
          <div class="mx-auto w-full max-w-[100rem] px-4 py-5 md:px-8 md:py-7">
            <RouterView />
          </div>
        </main>
      </div>
    </div>
    <CommandPalette v-model:open="palette" :items="items" :sign-out="signOut" />
  </div>
</template>
