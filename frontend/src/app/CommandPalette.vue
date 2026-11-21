<script setup lang="ts">
import { useQuery } from '@tanstack/vue-query'
import {
  Bug,
  CornerDownLeft,
  FolderKanban,
  Layers3,
  LogOut,
  type LucideIcon,
  Monitor,
  Moon,
  Search,
  Sun,
  Waypoints,
} from 'lucide-vue-next'
import { DialogContent, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'
import { computed, nextTick, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { api, unwrap } from '@/shared/api/client'
import { setThemePreference, type ThemePreference } from '@/shared/lib/theme'
import { useScopeStore } from '@/shared/scope/store'
import { Kbd } from '@/shared/ui/kbd'

import { NAV_ICONS } from './icons'
import { type NavItem } from './navigation'

/** The pages there are to go to, and what signing out does. */
const props = defineProps<{ items: NavItem[]; signOut: () => Promise<void> }>()
const open = defineModel<boolean>('open', { default: false })

const { t } = useI18n()
const router = useRouter()
const scope = useScopeStore()
const projects = useQuery({
  queryKey: ['projects'],
  queryFn: () => unwrap(api.GET('/api/v1/projects')),
  enabled: open,
})

interface Command {
  id: string
  group: 'jump' | 'pages' | 'scope' | 'actions' | 'search'
  label: string
  icon: LucideIcon
  keys?: string[]
  /** Shown even when the words do not match it. */
  always?: boolean
  run: () => unknown
}

const query = ref('')
const active = ref(0)
const list = ref<HTMLElement | null>(null)
const listId = useId()

const THEMES: { value: ThemePreference; icon: LucideIcon }[] = [
  { value: 'system', icon: Monitor },
  { value: 'light', icon: Sun },
  { value: 'dark', icon: Moon },
]

function matches(label: string, words: string[]): boolean {
  const text = label.toLowerCase()
  return words.every((word) => text.includes(word))
}

const commands = computed<Command[]>(() => {
  const q = query.value.trim()
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  const all: Command[] = []
  const issue = /^#?(\d{1,12})$/.exec(q)
  if (issue) {
    all.push({
      id: 'issue',
      group: 'jump',
      label: t('palette.openIssue', { id: issue[1] }),
      icon: Bug,
      always: true,
      run: () => router.push(`/issues/${issue[1]}`),
    })
  }
  const trace = q.replaceAll('-', '').toLowerCase()
  if (/^[0-9a-f]{32}$/.test(trace)) {
    all.push({
      id: 'trace',
      group: 'jump',
      label: t('palette.openTrace', { id: trace.slice(0, 8) }),
      icon: Waypoints,
      always: true,
      run: () => router.push(`/traces/${trace}`),
    })
  }
  for (const item of props.items) {
    all.push({
      id: `page-${item.name}`,
      group: 'pages',
      label: t(`nav.${item.name}`),
      icon: NAV_ICONS[item.icon] ?? Layers3,
      keys: item.key ? ['G', item.key.toUpperCase()] : undefined,
      run: () => router.push(item.path),
    })
  }
  all.push({
    id: 'project-all',
    group: 'scope',
    label: t('scope.allProjects'),
    icon: FolderKanban,
    run: () => scope.choose(null),
  })
  for (const project of projects.data.value ?? []) {
    all.push({
      id: `project-${project.id}`,
      group: 'scope',
      label: t('palette.project', { name: project.name }),
      icon: FolderKanban,
      run: () => scope.choose(project.id),
    })
    if (project.id !== scope.project) continue
    for (const stage of project.environments) {
      all.push({
        id: `stage-${stage.id}`,
        group: 'scope',
        label: t('palette.stage', { project: project.name, name: stage.name }),
        icon: Layers3,
        run: () => (scope.environment = stage.id),
      })
    }
  }
  for (const theme of THEMES) {
    all.push({
      id: `theme-${theme.value}`,
      group: 'actions',
      label: t('palette.theme', { theme: t(`profile.themes.${theme.value}`) }),
      icon: theme.icon,
      run: () => setThemePreference(theme.value),
    })
  }
  all.push({
    id: 'sign-out',
    group: 'actions',
    label: t('auth.signOut'),
    icon: LogOut,
    run: () => props.signOut(),
  })
  if (q) {
    all.push(
      {
        id: 'search-logs',
        group: 'search',
        label: t('palette.searchLogs', { q }),
        icon: Search,
        always: true,
        run: () => router.push({ path: '/logs', query: { q } }),
      },
      {
        id: 'search-issues',
        group: 'search',
        label: t('palette.searchIssues', { q }),
        icon: Search,
        always: true,
        run: () => router.push({ path: '/issues', query: { q, status: 'all' } }),
      },
    )
  }
  return all.filter((c) => c.always || matches(c.label, words))
})

const GROUPS = ['jump', 'pages', 'scope', 'actions', 'search'] as const
const groups = computed(() =>
  GROUPS.map((group) => ({
    group,
    commands: commands.value.filter((c) => c.group === group),
  })).filter((g) => g.commands.length > 0),
)
/** The commands in the order shown, for the arrow keys. */
const ordered = computed(() => groups.value.flatMap((g) => g.commands))

watch(query, () => (active.value = 0))
watch(open, (now) => {
  if (now) {
    query.value = ''
    active.value = 0
  }
})

function move(by: number): void {
  const count = ordered.value.length
  if (count === 0) return
  active.value = (active.value + by + count) % count
  void nextTick(() =>
    list.value?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' }),
  )
}

async function run(command: Command | undefined): Promise<void> {
  if (!command) return
  open.value = false
  await command.run()
}
</script>

<!-- Everything there is to go to or do, found by typing: ⌘K. -->
<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-[1000] animate-fade-in bg-black/30 backdrop-blur-[3px] dark:bg-black/50"
      />
      <DialogContent
        class="fixed top-[12vh] left-1/2 z-[1001] flex max-h-[min(32rem,76vh)] w-[calc(100%-2rem)] max-w-xl -translate-x-1/2 animate-pop-in flex-col overflow-hidden rounded-2xl bg-popover text-popover-foreground shadow-pop"
        :aria-describedby="undefined"
      >
        <DialogTitle class="sr-only">{{ t('palette.title') }}</DialogTitle>
        <div class="flex items-center gap-2.5 border-b px-4">
          <Search class="size-4 shrink-0 text-muted-foreground" />
          <input
            v-model="query"
            class="h-12 w-full bg-transparent text-[15px] outline-none placeholder:text-muted-foreground"
            :placeholder="t('palette.placeholder')"
            role="combobox"
            aria-expanded="true"
            :aria-controls="listId"
            :aria-activedescendant="
              ordered[active] ? `${listId}-${ordered[active]!.id}` : undefined
            "
            :aria-label="t('palette.title')"
            autocomplete="off"
            spellcheck="false"
            @keydown.down.prevent="move(1)"
            @keydown.up.prevent="move(-1)"
            @keydown.enter.prevent="run(ordered[active])"
          />
          <Kbd>esc</Kbd>
        </div>
        <div
          :id="listId"
          ref="list"
          role="listbox"
          :aria-label="t('palette.title')"
          class="min-h-0 flex-1 overflow-y-auto p-2"
        >
          <p
            v-if="ordered.length === 0"
            class="px-3 py-8 text-center text-sm text-muted-foreground"
          >
            {{ t('palette.empty') }}
          </p>
          <div
            v-for="g in groups"
            :key="g.group"
            role="group"
            :aria-label="t(`palette.groups.${g.group}`)"
          >
            <p class="px-2.5 pt-2 pb-1 text-[11px] font-medium text-muted-foreground">
              {{ t(`palette.groups.${g.group}`) }}
            </p>
            <div
              v-for="command in g.commands"
              :id="`${listId}-${command.id}`"
              :key="command.id"
              role="option"
              :aria-selected="ordered[active]?.id === command.id"
              class="flex h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-sm aria-selected:bg-accent aria-selected:text-accent-foreground"
              @mousemove="active = ordered.indexOf(command)"
              @click="run(command)"
            >
              <component :is="command.icon" class="size-4 shrink-0 text-muted-foreground" />
              <span class="min-w-0 flex-1 truncate" data-label>{{ command.label }}</span>
              <span v-if="command.keys" class="flex gap-1">
                <Kbd v-for="key in command.keys" :key="key">{{ key }}</Kbd>
              </span>
              <CornerDownLeft
                v-else-if="ordered[active]?.id === command.id"
                class="size-3.5 text-muted-foreground"
              />
            </div>
          </div>
        </div>
        <div
          class="flex items-center gap-3 border-t bg-muted/40 px-4 py-2 text-[11px] text-muted-foreground"
        >
          <span class="flex items-center gap-1"
            ><Kbd>↑</Kbd><Kbd>↓</Kbd> {{ t('palette.move') }}</span
          >
          <span class="flex items-center gap-1"
            ><Kbd><CornerDownLeft class="size-3" /></Kbd> {{ t('palette.open') }}</span
          >
          <span class="ml-auto flex items-center gap-1"><Kbd>/</Kbd> {{ t('palette.slash') }}</span>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
