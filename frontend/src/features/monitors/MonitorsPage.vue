<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatRelative } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { EmptyState, PageHeader } from '@/shared/ui/page'
import { Loading } from '@/shared/ui/skeleton'

import { useMonitors, variant } from './api'

const { t } = useI18n()
const scope = useScopeStore()
const names = useProjectNames()
const monitors = useMonitors(
  () => scope.project,
  () => scope.environment,
)
</script>

<template>
  <div class="grid content-start gap-4">
    <PageHeader :title="t('monitors.title')" :description="t('monitors.description')" />
    <Alert v-if="monitors.error.value" variant="destructive">{{
      problemMessage(monitors.error.value)
    }}</Alert>
    <Loading v-if="monitors.isPending.value" />
    <EmptyState
      v-else-if="(monitors.data.value ?? []).length === 0"
      :title="t('monitors.empty')"
      :description="t('monitors.emptyHint')"
    />
    <ul
      v-else
      class="divide-y overflow-hidden rounded-xl bg-card shadow-panel"
      :aria-label="t('monitors.title')"
    >
      <li
        v-for="monitor in monitors.data.value"
        :key="monitor.id"
        class="grid gap-1 px-4 py-3 md:grid-cols-[minmax(0,1fr)_9rem_10rem_10rem] md:items-center md:gap-3"
      >
        <div class="grid min-w-0 gap-1">
          <div class="flex min-w-0 items-center gap-2">
            <Badge :variant="variant(monitor.health)">{{
              t(`monitors.health.${monitor.health}`)
            }}</Badge>
            <RouterLink
              :to="`/monitors/${monitor.id}`"
              class="truncate font-medium hover:underline"
              >{{ monitor.name }}</RouterLink
            >
          </div>
          <p class="text-xs text-muted-foreground">
            <template v-if="scope.project === null"
              >{{ names.project(monitor.project_id) }} · </template
            >{{ names.environment(monitor.environment_id) }}
          </p>
        </div>
        <span class="font-mono text-xs text-muted-foreground">
          {{ monitor.schedule ?? t('monitors.noSchedule') }}
          <template v-if="monitor.timezone && monitor.timezone !== 'UTC'">
            · {{ monitor.timezone }}</template
          >
        </span>
        <span class="text-sm" :title="formatDateTime(monitor.last_check_in)">
          <span class="text-xs text-muted-foreground">{{ t('monitors.lastRun') }}</span>
          {{ formatRelative(monitor.last_check_in) }}
        </span>
        <span class="text-sm" :title="formatDateTime(monitor.next_run)">
          <span class="text-xs text-muted-foreground">{{ t('monitors.nextRun') }}</span>
          {{ formatRelative(monitor.next_run) }}
        </span>
      </li>
    </ul>
  </div>
</template>
