<script setup lang="ts">
import { Pause, Play, RefreshCw, Search } from 'lucide-vue-next'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatTime } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, PageHeader } from '@/shared/ui/page'

import { type LogEntry, openStream, useHistogram, useLogPages, useSources } from './api'
import { fromQuery, LEVELS, type LevelName, RANGES, toParams, toQuery } from './filters'
import LevelBadge from './LevelBadge.vue'
import LogDetail from './LogDetail.vue'
import LogHistogram from './LogHistogram.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const scope = useScopeStore()
const names = useProjectNames()

const filters = ref(fromQuery(route.query))
const typed = ref(filters.value.q)
watch(
  () => route.query,
  (query) => {
    filters.value = fromQuery(query)
    typed.value = filters.value.q
  },
)
function update(changes: Partial<typeof filters.value>): void {
  void router.replace({ query: toQuery({ ...filters.value, ...changes }) })
}

// The time range is fixed when the filters change, so pages stay stable.
const now = ref(new Date())
watch([filters, () => scope.project, () => scope.environment], () => (now.value = new Date()), {
  deep: true,
})
const params = computed(() =>
  toParams(filters.value, { project: scope.project, environment: scope.environment }, now.value),
)
const pages = useLogPages(params)
const histogram = useHistogram(params)
const sources = useSources(() => scope.project)
const entries = computed(() => pages.data.value?.pages.flatMap((p) => p.entries) ?? [])
const services = computed(
  () => [...new Set((sources.data.value ?? []).map((s) => s.service).filter(Boolean))] as string[],
)
const hosts = computed(
  () => [...new Set((sources.data.value ?? []).map((s) => s.host).filter(Boolean))] as string[],
)
const several = computed(() => scope.project === null)

const selected = ref<LogEntry | null>(null)

// The live view: entries as they are written, newest on top.
const live = ref(false)
const arrived = ref<LogEntry[]>([])
const missed = ref(0)
let stop: (() => void) | null = null
function listen(): void {
  stop?.()
  stop = null
  arrived.value = []
  missed.value = 0
  if (!live.value) return
  stop = openStream(
    params.value,
    (entry) => {
      arrived.value = [entry, ...arrived.value].slice(0, 500)
    },
    (count) => (missed.value += count),
  )
}
watch([live, params], listen)
onBeforeUnmount(() => stop?.())
const shown = computed(() => (live.value ? [...arrived.value, ...entries.value] : entries.value))

function search(): void {
  update({ q: typed.value })
}

function toggleLevel(level: LevelName): void {
  const levels = filters.value.levels.includes(level)
    ? filters.value.levels.filter((l) => l !== level)
    : [...filters.value.levels, level]
  update({ levels })
}

function refresh(): void {
  now.value = new Date()
}
</script>

<template>
  <div class="grid gap-4" :class="selected && 'lg:grid-cols-[1fr_26rem]'">
    <div class="grid min-w-0 content-start gap-4">
      <PageHeader :title="t('logs.title')">
        <ScopePicker />
      </PageHeader>

      <form class="flex flex-wrap items-center gap-2" @submit.prevent="search">
        <div class="relative min-w-64 flex-1">
          <Search
            class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            v-model="typed"
            class="pl-8 font-mono"
            :placeholder="t('logs.searchHint')"
            :aria-label="t('logs.search')"
          />
        </div>
        <NativeSelect
          :model-value="filters.range"
          class="w-36"
          :aria-label="t('logs.range')"
          @update:model-value="(range) => update({ range: range as typeof filters.range })"
        >
          <option v-for="range in RANGES" :key="range" :value="range">
            {{ t(`logs.ranges.${range}`) }}
          </option>
          <option v-if="filters.range === 'custom'" value="custom">
            {{ t('logs.ranges.custom') }}
          </option>
        </NativeSelect>
        <Button type="submit" variant="outline">{{ t('logs.searchButton') }}</Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          :aria-label="t('logs.refresh')"
          @click="refresh"
          ><RefreshCw
        /></Button>
        <Button type="button" :variant="live ? 'default' : 'outline'" @click="live = !live">
          <component :is="live ? Pause : Play" />{{ live ? t('logs.liveOn') : t('logs.liveOff') }}
        </Button>
      </form>

      <div class="flex flex-wrap items-center gap-2 text-sm">
        <button
          v-for="level in LEVELS"
          :key="level"
          type="button"
          class="rounded-full border px-2.5 py-0.5"
          :class="
            filters.levels.includes(level)
              ? 'border-primary bg-primary/10'
              : 'text-muted-foreground'
          "
          :aria-pressed="filters.levels.includes(level)"
          @click="toggleLevel(level)"
        >
          <LevelBadge :level="level" />
        </button>
        <NativeSelect
          v-if="services.length"
          :model-value="filters.services[0] ?? ''"
          class="w-40"
          :aria-label="t('logs.fields.service')"
          @update:model-value="(s) => update({ services: s ? [String(s)] : [] })"
        >
          <option value="">{{ t('logs.allServices') }}</option>
          <option v-for="service in services" :key="service" :value="service">{{ service }}</option>
        </NativeSelect>
        <NativeSelect
          v-if="hosts.length"
          :model-value="filters.hosts[0] ?? ''"
          class="w-44"
          :aria-label="t('logs.fields.host')"
          @update:model-value="(h) => update({ hosts: h ? [String(h)] : [] })"
        >
          <option value="">{{ t('logs.allHosts') }}</option>
          <option v-for="host in hosts" :key="host" :value="host">{{ host }}</option>
        </NativeSelect>
        <Badge v-if="filters.trace" variant="info" class="h-6 gap-1">
          {{ t('logs.trace') }}: <code>{{ filters.trace.slice(0, 12) }}…</code>
          <button
            type="button"
            :aria-label="t('logs.clearTrace')"
            @click="update({ trace: undefined })"
          >
            ✕
          </button>
        </Badge>
        <Badge v-if="filters.range === 'custom'" variant="info" class="h-6 gap-1">
          {{ formatDateTime(filters.from) }} – {{ formatDateTime(filters.to) }}
          <button
            type="button"
            :aria-label="t('logs.clearRange')"
            @click="update({ range: '24h' })"
          >
            ✕
          </button>
        </Badge>
      </div>

      <LogHistogram
        v-if="histogram.data.value && !filters.trace"
        :histogram="histogram.data.value"
        @select="(range) => update({ range: 'custom', ...range })"
      />

      <Alert v-if="pages.error.value" variant="destructive">{{
        problemMessage(pages.error.value)
      }}</Alert>
      <Alert v-if="missed > 0">{{ t('logs.missed', { count: missed }) }}</Alert>
      <p v-if="pages.isPending.value" class="text-sm text-muted-foreground">
        {{ t('common.loading') }}
      </p>
      <EmptyState
        v-else-if="shown.length === 0"
        :title="t('logs.empty')"
        :description="t('logs.emptyHint')"
      />
      <ol v-else class="divide-y rounded-lg border font-mono text-xs" :aria-label="t('logs.list')">
        <li
          v-for="entry in shown"
          :key="entry.id"
          class="grid cursor-pointer grid-cols-[6.5rem_4.5rem_minmax(0,1fr)] items-baseline gap-2 px-3 py-1.5 hover:bg-accent sm:grid-cols-[6.5rem_4.5rem_9rem_minmax(0,1fr)]"
          :class="selected?.id === entry.id && 'bg-accent'"
          @click="selected = entry"
        >
          <time
            :datetime="entry.time"
            :title="formatDateTime(entry.time)"
            class="text-muted-foreground"
          >
            {{ formatTime(entry.time) }}
          </time>
          <LevelBadge :level="entry.level" />
          <span class="hidden truncate text-muted-foreground sm:inline" :title="entry.host ?? ''">
            <template v-if="several">{{ names.project(entry.project_id) }} · </template
            >{{ entry.service ?? '–' }}
          </span>
          <span class="truncate" :title="entry.message">{{ entry.message }}</span>
        </li>
      </ol>
      <div v-if="pages.hasNextPage.value" class="flex justify-center">
        <Button
          variant="outline"
          :disabled="pages.isFetchingNextPage.value"
          @click="pages.fetchNextPage()"
        >
          {{ t('logs.more') }}
        </Button>
      </div>
    </div>

    <LogDetail
      v-if="selected"
      :entry="selected"
      class="lg:sticky lg:top-4 lg:max-h-[calc(100dvh-2rem)] lg:overflow-y-auto"
      @close="selected = null"
      @trace="(trace) => update({ trace })"
      @filter="(term) => update({ q: [filters.q, term].filter(Boolean).join(' ') })"
    />
  </div>
</template>
