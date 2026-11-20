<script setup lang="ts">
import { ArrowLeft, Link2 } from 'lucide-vue-next'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatDuration, formatTime } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { Alert } from '@/shared/ui/alert'
import { buttonVariants } from '@/shared/ui/button'
import { LevelBadge } from '@/shared/ui/level'
import { EmptyState } from '@/shared/ui/page'
import { buildTimeline, RequestTimeline } from '@/shared/trace'

import { useTrace } from './api'

const props = defineProps<{ id: string }>()
const { t } = useI18n()
const names = useProjectNames()
const trace = useTrace(() => props.id)
const view = computed(() => buildTimeline(trace.data.value?.transactions ?? []))
const first = computed(() => trace.data.value?.transactions[0])
const empty = computed(() => {
  const data = trace.data.value
  return data && data.transactions.length + data.logs.length + data.events.length === 0
})
</script>

<template>
  <div class="grid content-start gap-6">
    <RouterLink
      to="/performance"
      class="flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      ><ArrowLeft class="size-4" />{{ t('performance.title') }}</RouterLink
    >
    <header class="grid gap-1">
      <h1 class="text-xl font-semibold tracking-tight">{{ t('traces.title') }}</h1>
      <p class="font-mono text-xs break-all text-muted-foreground">{{ id }}</p>
      <p v-if="first" class="text-sm text-muted-foreground">
        {{ names.project(first.project_id) }} / {{ names.environment(first.environment_id) }} ·
        {{ formatDateTime(first.start) }} · {{ formatDuration(view.total / 1000) }}
      </p>
    </header>
    <Alert v-if="trace.error.value" variant="destructive">{{
      problemMessage(trace.error.value)
    }}</Alert>
    <p v-else-if="trace.isPending.value" class="text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </p>
    <EmptyState v-else-if="empty" :title="t('traces.empty')" :description="t('traces.emptyHint')" />
    <template v-else-if="trace.data.value">
      <section v-if="view.roots.length" class="grid gap-2">
        <h2 class="font-medium">{{ t('traces.waterfall') }}</h2>
        <RequestTimeline :transactions="trace.data.value.transactions" />
      </section>

      <section v-if="trace.data.value.events.length" class="grid gap-2">
        <h2 class="font-medium">{{ t('traces.errors') }}</h2>
        <ul class="divide-y rounded-lg border text-sm">
          <li
            v-for="event in trace.data.value.events"
            :key="event.event_id"
            class="flex items-center gap-2 px-3 py-1.5"
          >
            <LevelBadge :level="event.level" />
            <RouterLink :to="`/issues/${event.issue_id}`" class="truncate hover:underline">{{
              event.message
            }}</RouterLink>
          </li>
        </ul>
      </section>

      <section v-if="trace.data.value.logs.length" class="grid gap-2">
        <div class="flex items-center justify-between gap-2">
          <h2 class="font-medium">{{ t('traces.logs') }}</h2>
          <RouterLink
            :to="{ path: '/logs', query: { trace: id } }"
            :class="buttonVariants({ variant: 'outline', size: 'sm' })"
            ><Link2 />{{ t('traces.openLogs') }}</RouterLink
          >
        </div>
        <ol class="divide-y rounded-lg border font-mono text-xs">
          <li
            v-for="entry in trace.data.value.logs"
            :key="entry.id"
            class="grid grid-cols-[6.5rem_4.5rem_minmax(0,1fr)] items-baseline gap-2 px-3 py-1.5"
          >
            <time :datetime="entry.time" class="text-muted-foreground">{{
              formatTime(entry.time)
            }}</time>
            <LevelBadge :level="entry.level" />
            <span class="truncate" :title="entry.message">{{ entry.message }}</span>
          </li>
        </ol>
      </section>
    </template>
  </div>
</template>
