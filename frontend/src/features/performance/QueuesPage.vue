<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { type LocationQuery, RouterLink, useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDuration, formatNumber } from '@/shared/lib/format'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, PageHeader } from '@/shared/ui/page'
import { SeriesChart } from '@/shared/ui/series-chart'

import { type PerfRange, RANGES, rangeParams, useQueues, useSeries } from './api'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const scope = useScopeStore()

function one(query: LocationQuery, key: string): string {
  const value = query[key]
  return (Array.isArray(value) ? value[0] : value) ?? ''
}
const range = computed(() => {
  const value = one(route.query, 'range')
  return ((RANGES as readonly string[]).includes(value) ? value : '24h') as PerfRange
})
const selected = computed(() => one(route.query, 'queue') || null)

function update(changes: { range?: string; queue?: string | null }): void {
  const query: Record<string, string> = {}
  const next = { range: range.value, queue: selected.value, ...changes }
  if (next.range !== '24h') query.range = next.range
  if (next.queue) query.queue = next.queue
  void router.replace({ query })
}

// The range ends when the filters change, so the numbers stay put.
const now = ref(new Date())
watch([range, () => scope.project, () => scope.environment], () => (now.value = new Date()))
/** The range's parameters, for one kind of queue measurement. */
const params = (kind: string) => ({
  ...rangeParams(
    {
      kind: 'job',
      range: range.value,
      sort: 'count',
      project: scope.project,
      environment: scope.environment,
    },
    now.value,
  ),
  kind,
})
const queues = useQueues(computed(() => params('queue_process')))
const row = computed(() => queues.data.value?.find((q) => q.name === selected.value))
const runs = useSeries(
  computed(() => params('queue_process')),
  selected,
)
const waits = useSeries(
  computed(() => params('queue_wait')),
  selected,
)

const share = (part: number, of: number) =>
  of === 0 ? '–' : `${Math.round((part / of) * 1000) / 10} %`
const ms = (value: number | undefined | null) =>
  value === undefined || value === null ? '–' : formatDuration(value)

const processedPoints = computed(() =>
  (runs.data.value?.buckets ?? []).map((b) => ({
    start: b.start,
    value: b.count,
    part: b.flagged,
  })),
)
const runPoints = computed(() =>
  (runs.data.value?.buckets ?? []).map((b) => ({ start: b.start, value: b.p95_ms })),
)
const waitPoints = computed(() =>
  (waits.data.value?.buckets ?? []).map((b) => ({ start: b.start, value: b.p95_ms })),
)
</script>

<template>
  <div class="grid gap-4" :class="selected && 'xl:grid-cols-[minmax(0,1fr)_30rem]'">
    <div class="grid min-w-0 content-start gap-4">
      <PageHeader :title="t('queues.title')" :description="t('queues.description')">
        <ScopePicker />
      </PageHeader>

      <div class="flex flex-wrap items-center gap-2">
        <NativeSelect
          :model-value="range"
          class="w-40"
          :aria-label="t('performance.range')"
          @update:model-value="(value) => update({ range: String(value) })"
        >
          <option v-for="option in RANGES" :key="option" :value="option">
            {{ t(`performance.ranges.${option}`) }}
          </option>
        </NativeSelect>
        <RouterLink
          :to="{ path: '/performance', query: { kind: 'job' } }"
          class="text-sm text-muted-foreground hover:text-foreground hover:underline"
          >{{ t('queues.jobs') }}</RouterLink
        >
      </div>

      <Alert v-if="queues.error.value" variant="destructive">{{
        problemMessage(queues.error.value)
      }}</Alert>
      <p v-if="queues.isPending.value" class="text-sm text-muted-foreground">
        {{ t('common.loading') }}
      </p>
      <EmptyState
        v-else-if="(queues.data.value ?? []).length === 0"
        :title="t('queues.empty')"
        :description="t('queues.emptyHint')"
      />
      <div v-else class="overflow-x-auto rounded-lg border">
        <table class="w-full text-sm" :aria-label="t('queues.title')">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="px-3 py-2 text-left font-medium">{{ t('queues.queue') }}</th>
              <th class="px-3 py-2 text-right font-medium">{{ t('queues.processed') }}</th>
              <th class="px-3 py-2 text-right font-medium">{{ t('queues.failed') }}</th>
              <th class="hidden px-3 py-2 text-right font-medium sm:table-cell">
                {{ t('queues.retries') }}
              </th>
              <th class="px-3 py-2 text-right font-medium">{{ t('queues.waitP95') }}</th>
              <th class="px-3 py-2 text-right font-medium">{{ t('queues.runP95') }}</th>
              <th class="hidden px-3 py-2 text-right font-medium md:table-cell">
                {{ t('queues.published') }}
              </th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr
              v-for="queue in queues.data.value"
              :key="queue.name"
              class="cursor-pointer hover:bg-accent/50"
              :class="selected === queue.name && 'bg-accent'"
              @click="update({ queue: queue.name })"
            >
              <td class="px-3 py-1.5 font-mono text-xs">{{ queue.name }}</td>
              <td class="px-3 py-1.5 text-right tabular-nums">
                {{ formatNumber(queue.processed) }}
              </td>
              <td
                class="px-3 py-1.5 text-right tabular-nums"
                :class="queue.failed > 0 && 'text-destructive'"
              >
                {{ share(queue.failed, queue.processed) }}
              </td>
              <td class="hidden px-3 py-1.5 text-right tabular-nums sm:table-cell">
                {{ share(queue.retries, queue.processed) }}
              </td>
              <td class="px-3 py-1.5 text-right tabular-nums">{{ ms(queue.wait?.p95_ms) }}</td>
              <td class="px-3 py-1.5 text-right tabular-nums">
                {{ ms(queue.processing?.p95_ms) }}
              </td>
              <td class="hidden px-3 py-1.5 text-right tabular-nums md:table-cell">
                {{ formatNumber(queue.published) }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <aside v-if="selected" class="grid content-start gap-4 rounded-lg border p-4">
      <header class="flex items-start justify-between gap-2">
        <h2 class="font-mono text-sm font-medium break-all">{{ selected }}</h2>
        <button
          type="button"
          class="text-sm text-muted-foreground hover:text-foreground"
          @click="update({ queue: null })"
        >
          {{ t('common.close') }}
        </button>
      </header>
      <dl v-if="row" class="grid grid-cols-2 gap-3 text-sm">
        <div>
          <dt class="text-muted-foreground">{{ t('queues.waitAverage') }}</dt>
          <dd class="tabular-nums">{{ ms(row.wait?.average_ms) }}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground">{{ t('queues.runAverage') }}</dt>
          <dd class="tabular-nums">{{ ms(row.processing?.average_ms) }}</dd>
        </div>
      </dl>
      <SeriesChart
        :points="processedPoints"
        :title="t('queues.chart.processed')"
        :label="t('queues.processed')"
        :part-label="t('queues.failed')"
        :format="formatNumber"
      />
      <SeriesChart
        :points="waitPoints"
        :title="t('queues.chart.wait')"
        label="p95"
        :format="formatDuration"
      />
      <SeriesChart
        :points="runPoints"
        :title="t('queues.chart.run')"
        label="p95"
        :format="formatDuration"
      />
    </aside>
  </div>
</template>
