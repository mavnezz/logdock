<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatBytes, formatDay, formatNumber } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import { useSessionStore } from '@/shared/session/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { PageHeader } from '@/shared/ui/page'

import { useStorage } from './api'
import StorageDays from './StorageDays.vue'

const { t } = useI18n()
const storage = useStorage()
const names = useProjectNames()
const session = useSessionStore()
const data = computed(() => storage.data.value)

const KINDS = ['logs', 'events', 'traces'] as const

const used = computed(() => (data.value ? data.value.segment_bytes + data.value.database_bytes : 0))
/** The share of the limit the segments take, for the bar. */
const limitShare = computed(() => {
  const d = data.value
  if (!d || d.max_storage_bytes === 0) return null
  return Math.min(1, d.segment_bytes / d.max_storage_bytes)
})

/** Each project's stage with what it takes of each kind, the largest first. */
const stages = computed(() => {
  const byStage = new Map<
    string,
    { project: number; environment: number; sizes: Record<string, number>; total: number }
  >()
  for (const s of data.value?.stages ?? []) {
    const key = `${s.project_id}-${s.environment_id}`
    const stage = byStage.get(key) ?? {
      project: s.project_id,
      environment: s.environment_id,
      sizes: {},
      total: 0,
    }
    stage.sizes[s.kind] = (stage.sizes[s.kind] ?? 0) + s.bytes
    stage.total += s.bytes
    byStage.set(key, stage)
  }
  return [...byStage.values()].sort((a, b) => b.total - a.total)
})

/** Days a stage keeps a kind: its own, or the server's. */
function keptBy(project: number, environment: number, kind: (typeof KINDS)[number]) {
  const own = names.settings(project)?.retention?.find((r) => r.environment_id === environment)?.[
    `${kind}_days`
  ]
  const server = data.value?.kinds.find((k) => k.kind === kind)?.retention_days
  return { days: own ?? server ?? 0, own: own != null }
}
</script>

<template>
  <div class="grid content-start gap-6">
    <PageHeader :title="t('storage.title')" :description="t('storage.description')" />

    <Alert v-if="storage.error.value" variant="destructive">{{
      problemMessage(storage.error.value)
    }}</Alert>
    <p v-else-if="!data" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <template v-else>
      <dl
        class="grid grid-cols-2 gap-px overflow-hidden rounded-lg border bg-border lg:grid-cols-4"
      >
        <div class="grid gap-1 bg-card p-4">
          <dt class="text-xs text-muted-foreground">{{ t('storage.used') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">{{ formatBytes(used) }}</dd>
          <dd class="text-xs text-muted-foreground">
            {{ t('storage.database', { size: formatBytes(data.database_bytes) }) }}
          </dd>
        </div>
        <div class="grid gap-1 bg-card p-4">
          <dt class="text-xs text-muted-foreground">{{ t('storage.limit') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">
            {{
              data.max_storage_bytes ? formatBytes(data.max_storage_bytes) : t('storage.noLimit')
            }}
          </dd>
          <dd v-if="limitShare !== null" class="grid gap-1">
            <span
              class="h-1.5 overflow-hidden rounded-full bg-muted"
              role="meter"
              :aria-valuenow="Math.round(limitShare * 100)"
              aria-valuemin="0"
              aria-valuemax="100"
              :aria-label="t('storage.limit')"
            >
              <span
                class="block h-full rounded-full"
                :class="limitShare > 0.9 ? 'bg-destructive' : 'bg-primary'"
                :style="{ width: `${limitShare * 100}%` }"
              />
            </span>
            <span class="text-xs text-muted-foreground">{{
              t('storage.ofLimit', { share: Math.round(limitShare * 100) })
            }}</span>
          </dd>
        </div>
        <div class="grid gap-1 bg-card p-4">
          <dt class="text-xs text-muted-foreground">{{ t('storage.free') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">
            {{ data.free_bytes != null ? formatBytes(data.free_bytes) : '–' }}
          </dd>
          <dd class="text-xs text-muted-foreground">
            {{ t('storage.reserve', { size: formatBytes(data.min_free_bytes) }) }}
          </dd>
        </div>
        <div class="grid gap-1 bg-card p-4">
          <dt class="text-xs text-muted-foreground">{{ t('storage.perDayAverage') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">
            {{ data.forecast ? formatBytes(data.forecast.daily_bytes) : '–' }}
          </dd>
          <dd class="text-xs text-muted-foreground">
            {{
              data.forecast
                ? t('storage.settles', { size: formatBytes(data.forecast.settled_bytes) })
                : t('storage.noForecast')
            }}
          </dd>
        </div>
      </dl>

      <Alert v-if="data.forecast?.disk_in_days != null" variant="destructive">{{
        t('storage.diskFull', { n: data.forecast.disk_in_days }, data.forecast.disk_in_days)
      }}</Alert>
      <Alert v-if="data.forecast?.limit_in_days != null">{{
        t('storage.limitReached', { n: data.forecast.limit_in_days }, data.forecast.limit_in_days)
      }}</Alert>

      <StorageDays v-if="data.segments.length" :segments="data.segments" />

      <section class="grid gap-2">
        <h2 class="text-sm font-medium">{{ t('storage.byKind') }}</h2>
        <div class="overflow-x-auto rounded-lg border">
          <table class="w-full text-sm">
            <thead class="bg-muted/40 text-xs text-muted-foreground">
              <tr>
                <th class="px-3 py-2 text-left font-medium">{{ t('storage.kind') }}</th>
                <th class="px-3 py-2 text-right font-medium">{{ t('storage.kept') }}</th>
                <th class="px-3 py-2 text-right font-medium">{{ t('storage.oldest') }}</th>
                <th class="px-3 py-2 text-right font-medium">{{ t('storage.size') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y">
              <tr v-for="kind in data.kinds" :key="kind.kind">
                <td class="px-3 py-2">{{ t(`storage.kinds.${kind.kind}`) }}</td>
                <td class="px-3 py-2 text-right tabular-nums">
                  {{ t('storage.days', { n: kind.retention_days }, kind.retention_days) }}
                </td>
                <td class="px-3 py-2 text-right tabular-nums">
                  {{ kind.oldest ? formatDay(kind.oldest) : '–' }}
                </td>
                <td class="px-3 py-2 text-right tabular-nums">{{ formatBytes(kind.bytes) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="grid gap-2">
        <h2 class="text-sm font-medium">{{ t('storage.byStage') }}</h2>
        <p class="text-xs text-muted-foreground">{{ t('storage.byStageHint') }}</p>
        <p v-if="stages.length === 0" class="text-sm text-muted-foreground">
          {{ t('storage.notMeasured') }}
        </p>
        <div v-else class="overflow-x-auto rounded-lg border">
          <table class="w-full text-sm" :aria-label="t('storage.byStage')">
            <thead class="bg-muted/40 text-xs text-muted-foreground">
              <tr>
                <th class="px-3 py-2 text-left font-medium">{{ t('storage.stage') }}</th>
                <th v-for="kind in KINDS" :key="kind" class="px-3 py-2 text-right font-medium">
                  {{ t(`storage.kinds.${kind}`) }}
                </th>
                <th class="px-3 py-2 text-right font-medium">{{ t('storage.total') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y">
              <tr v-for="stage in stages" :key="`${stage.project}-${stage.environment}`">
                <td class="px-3 py-2">
                  <RouterLink
                    v-if="session.isAdmin"
                    :to="{ path: `/projects/${stage.project}`, query: { tab: 'settings' } }"
                    class="hover:underline"
                    >{{ names.project(stage.project) }} /
                    {{ names.environment(stage.environment) }}</RouterLink
                  >
                  <template v-else
                    >{{ names.project(stage.project) }} /
                    {{ names.environment(stage.environment) }}</template
                  >
                </td>
                <td v-for="kind in KINDS" :key="kind" class="px-3 py-2 text-right tabular-nums">
                  <span class="block">{{ formatBytes(stage.sizes[kind] ?? 0) }}</span>
                  <span class="text-xs text-muted-foreground">
                    {{
                      t(
                        'storage.days',
                        { n: keptBy(stage.project, stage.environment, kind).days },
                        keptBy(stage.project, stage.environment, kind).days,
                      )
                    }}
                    <Badge
                      v-if="keptBy(stage.project, stage.environment, kind).own"
                      variant="secondary"
                      >{{ t('storage.own') }}</Badge
                    >
                  </span>
                </td>
                <td class="px-3 py-2 text-right font-medium tabular-nums">
                  {{ formatBytes(stage.total) }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ t('storage.rows', { n: formatNumber(data.stages.reduce((s, x) => s + x.rows, 0)) }) }}
        </p>
      </section>
    </template>
  </div>
</template>
