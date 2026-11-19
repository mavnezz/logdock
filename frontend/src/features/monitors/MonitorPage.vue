<script setup lang="ts">
import { ArrowLeft, Trash2 } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatDuration, formatRelative } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { useSessionStore } from '@/shared/session/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ConfirmDialog } from '@/shared/ui/confirm-dialog'

import { type CheckIn, useDeleteMonitor, useMonitor, variant } from './api'

const props = defineProps<{ id: string }>()
const { t } = useI18n()
const router = useRouter()
const session = useSessionStore()
const names = useProjectNames()
const id = computed(() => Number(props.id))
const detail = useMonitor(id)
const remove = useDeleteMonitor(id)
const monitor = computed(() => detail.data.value?.monitor)
const confirmDelete = ref(false)

/** The latest runs as a strip, oldest left: a dot per run. */
const strip = computed(() => [...(detail.data.value?.check_ins ?? [])].slice(0, 60).reverse())
const dot: Record<string, string> = {
  ok: 'bg-level-debug',
  error: 'bg-level-error',
  timeout: 'bg-level-error',
  missed: 'bg-level-warn',
  in_progress: 'bg-level-info',
}

async function deleteMonitor(): Promise<void> {
  await remove.mutateAsync()
  await router.push('/monitors')
}

const duration = (run: CheckIn) =>
  run.duration_ms === null || run.duration_ms === undefined ? '–' : formatDuration(run.duration_ms)
</script>

<template>
  <div class="grid content-start gap-6">
    <RouterLink
      to="/monitors"
      class="flex w-fit items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      ><ArrowLeft class="size-4" />{{ t('monitors.title') }}</RouterLink
    >
    <Alert v-if="detail.error.value" variant="destructive">{{
      problemMessage(detail.error.value)
    }}</Alert>
    <p v-else-if="!monitor" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <template v-else>
      <header class="flex flex-wrap items-start justify-between gap-3">
        <div class="grid gap-1">
          <div class="flex items-center gap-2">
            <Badge :variant="variant(monitor.health)">{{
              t(`monitors.health.${monitor.health}`)
            }}</Badge>
            <span class="text-sm text-muted-foreground">
              {{ names.project(monitor.project_id) }} /
              {{ names.environment(monitor.environment_id) }}
            </span>
          </div>
          <h1 class="text-xl font-semibold tracking-tight">{{ monitor.name }}</h1>
          <p class="font-mono text-xs text-muted-foreground">{{ monitor.slug }}</p>
        </div>
        <Button
          v-if="session.isAdmin"
          size="sm"
          variant="ghost"
          class="text-destructive"
          @click="confirmDelete = true"
          ><Trash2 />{{ t('common.delete') }}</Button
        >
      </header>

      <dl class="grid grid-cols-2 gap-4 rounded-lg border p-4 text-sm sm:grid-cols-4">
        <div>
          <dt class="text-muted-foreground">{{ t('monitors.schedule') }}</dt>
          <dd class="font-mono">{{ monitor.schedule ?? t('monitors.noSchedule') }}</dd>
          <dd v-if="monitor.timezone" class="text-xs text-muted-foreground">
            {{ monitor.timezone }}
          </dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('monitors.lastRun') }}</dt>
          <dd :title="formatDateTime(monitor.last_check_in)">
            {{ formatRelative(monitor.last_check_in) }}
          </dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('monitors.nextRun') }}</dt>
          <dd :title="formatDateTime(monitor.next_run)">{{ formatRelative(monitor.next_run) }}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('monitors.limits') }}</dt>
          <dd
            v-if="monitor.margin_minutes !== null && monitor.margin_minutes !== undefined"
            class="text-xs"
          >
            {{
              t('monitors.limitsText', {
                margin: monitor.margin_minutes,
                runtime: monitor.max_runtime_minutes,
              })
            }}
          </dd>
        </div>
      </dl>

      <section v-if="strip.length" class="grid gap-2">
        <h2 class="font-medium">{{ t('monitors.recent') }}</h2>
        <div
          class="flex flex-wrap gap-1"
          role="img"
          :aria-label="t('monitors.stripLabel', { count: strip.length })"
        >
          <span
            v-for="run in strip"
            :key="run.check_in_id"
            class="size-3 rounded-sm"
            :class="dot[run.status]"
            :title="`${formatDateTime(run.started)} · ${t(`monitors.status.${run.status}`)}`"
          />
        </div>
        <p class="flex flex-wrap gap-3 text-xs text-muted-foreground">
          <span
            v-for="status in ['ok', 'error', 'missed', 'in_progress']"
            :key="status"
            class="inline-flex items-center gap-1.5"
          >
            <span class="size-2.5 rounded-sm" :class="dot[status]" aria-hidden="true" />{{
              t(`monitors.status.${status}`)
            }}
          </span>
        </p>
      </section>

      <section class="grid gap-2">
        <h2 class="font-medium">{{ t('monitors.runs') }}</h2>
        <table class="w-full text-sm">
          <thead class="border-b text-left text-xs text-muted-foreground">
            <tr>
              <th class="py-1.5 pr-3 font-medium">{{ t('monitors.started') }}</th>
              <th class="py-1.5 pr-3 font-medium">{{ t('monitors.statusLabel') }}</th>
              <th class="py-1.5 pr-3 text-right font-medium">{{ t('monitors.duration') }}</th>
              <th class="hidden py-1.5 font-medium sm:table-cell">{{ t('monitors.release') }}</th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr v-for="run in detail.data.value?.check_ins ?? []" :key="run.check_in_id">
              <td class="py-1.5 pr-3">{{ formatDateTime(run.started) }}</td>
              <td class="py-1.5 pr-3">
                <Badge :variant="variant(run.status)">{{
                  t(`monitors.status.${run.status}`)
                }}</Badge>
              </td>
              <td class="py-1.5 pr-3 text-right tabular-nums">{{ duration(run) }}</td>
              <td class="hidden py-1.5 font-mono text-xs sm:table-cell">
                <RouterLink
                  v-if="run.trace_id"
                  :to="`/traces/${run.trace_id}`"
                  class="hover:underline"
                  >{{ run.release ?? t('monitors.trace') }}</RouterLink
                >
                <template v-else>{{ run.release ?? '' }}</template>
              </td>
            </tr>
          </tbody>
        </table>
      </section>

      <ConfirmDialog
        v-model:open="confirmDelete"
        :title="t('monitors.deleteTitle')"
        :description="t('monitors.deleteDescription')"
        :confirm-label="t('common.delete')"
        :cancel-label="t('common.cancel')"
        destructive
        @confirm="deleteMonitor"
      />
    </template>
  </div>
</template>
