<script setup lang="ts">
import { X } from 'lucide-vue-next'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { type LocationQuery, RouterLink, useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatDuration, formatNumber, formatRelative } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, PageHeader } from '@/shared/ui/page'

import {
  KINDS,
  type PerfKind,
  type PerfRange,
  type PerfSort,
  RANGES,
  rangeParams,
  SORTS,
  useOverview,
  useSamples,
  useSeries,
} from './api'
import SeriesChart from './SeriesChart.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const scope = useScopeStore()
const names = useProjectNames()

function one(query: LocationQuery, key: string): string {
  const value = query[key]
  return (Array.isArray(value) ? value[0] : value) ?? ''
}
function pick<T extends string>(value: string, allowed: readonly T[], fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback
}

const kind = computed(() => pick<PerfKind>(one(route.query, 'kind'), KINDS, 'request'))
const range = computed(() => pick<PerfRange>(one(route.query, 'range'), RANGES, '24h'))
const sort = computed(() => pick<PerfSort>(one(route.query, 'sort'), SORTS, 'total_time'))
const selected = computed(() => one(route.query, 'name') || null)

function update(changes: Record<string, string | undefined>): void {
  const query: Record<string, string> = {}
  const next = {
    kind: kind.value,
    range: range.value,
    sort: sort.value,
    name: selected.value ?? undefined,
    ...changes,
  }
  if (next.kind !== 'request') query.kind = next.kind
  if (next.range !== '24h') query.range = next.range
  if (next.sort !== 'total_time') query.sort = next.sort
  if (next.name) query.name = next.name
  void router.replace({ query })
}

// The range ends when the filters change, so the numbers stay put.
const now = ref(new Date())
watch([kind, range, () => scope.project, () => scope.environment], () => (now.value = new Date()))
const params = computed(() =>
  rangeParams(
    {
      kind: kind.value,
      range: range.value,
      sort: sort.value,
      project: scope.project,
      environment: scope.environment,
    },
    now.value,
  ),
)
const overview = useOverview(params, sort)
const series = useSeries(params, selected)
const transactions = computed(() => ['request', 'job', 'command'].includes(kind.value))
const samples = useSamples(params, selected, transactions)
const row = computed(() => overview.data.value?.find((r) => r.name === selected.value))

/** Failures for most kinds, misses for the cache. */
const flaggedLabel = computed(() =>
  kind.value === 'cache' ? t('performance.misses') : t('performance.failures'),
)
const share = (part: number, count: number) =>
  count === 0 ? '–' : `${formatNumber(Math.round((part / count) * 1000) / 10)} %`
const countPoints = computed(() =>
  (series.data.value?.buckets ?? []).map((b) => ({
    start: b.start,
    value: b.count,
    part: b.flagged,
  })),
)
const p95Points = computed(() =>
  (series.data.value?.buckets ?? []).map((b) => ({ start: b.start, value: b.p95_ms })),
)
</script>

<template>
  <div class="grid gap-4" :class="selected && 'xl:grid-cols-[minmax(0,1fr)_30rem]'">
    <div class="grid min-w-0 content-start gap-4">
      <PageHeader :title="t('performance.title')" :description="t('performance.description')">
        <ScopePicker />
      </PageHeader>

      <div class="flex flex-wrap items-center gap-2">
        <div
          class="flex flex-wrap gap-1 rounded-lg border p-1"
          role="tablist"
          :aria-label="t('performance.kind')"
        >
          <button
            v-for="option in KINDS"
            :key="option"
            type="button"
            role="tab"
            class="rounded-md px-2.5 py-1 text-sm"
            :class="
              kind === option
                ? 'bg-accent font-medium'
                : 'text-muted-foreground hover:text-foreground'
            "
            :aria-selected="kind === option"
            @click="update({ kind: option, name: undefined })"
          >
            {{ t(`performance.kinds.${option}`) }}
          </button>
        </div>
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
        <NativeSelect
          :model-value="sort"
          class="w-44"
          :aria-label="t('performance.sort')"
          @update:model-value="(value) => update({ sort: String(value) })"
        >
          <option v-for="option in SORTS" :key="option" :value="option">
            {{ t(`performance.sorts.${option}`) }}
          </option>
        </NativeSelect>
      </div>

      <Alert v-if="overview.error.value" variant="destructive">{{
        problemMessage(overview.error.value)
      }}</Alert>
      <p v-if="overview.isPending.value" class="text-sm text-muted-foreground">
        {{ t('common.loading') }}
      </p>
      <EmptyState
        v-else-if="(overview.data.value ?? []).length === 0"
        :title="t('performance.empty')"
        :description="t('performance.emptyHint')"
      />
      <div v-else class="overflow-x-auto rounded-lg border">
        <table class="w-full text-sm" :aria-label="t(`performance.kinds.${kind}`)">
          <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
            <tr>
              <th class="px-3 py-2 text-left font-medium">{{ t('performance.name') }}</th>
              <th class="px-3 py-2 text-right font-medium">{{ t('performance.count') }}</th>
              <th class="px-3 py-2 text-right font-medium">{{ flaggedLabel }}</th>
              <th class="px-3 py-2 text-right font-medium">p50</th>
              <th class="px-3 py-2 text-right font-medium">p95</th>
              <th class="hidden px-3 py-2 text-right font-medium sm:table-cell">p99</th>
              <th class="hidden px-3 py-2 text-right font-medium md:table-cell">
                {{ t('performance.total') }}
              </th>
            </tr>
          </thead>
          <tbody class="divide-y">
            <tr
              v-for="item in overview.data.value"
              :key="item.name"
              class="cursor-pointer hover:bg-accent/50"
              :class="selected === item.name && 'bg-accent'"
              @click="update({ name: item.name })"
            >
              <td class="max-w-md truncate px-3 py-1.5 font-mono text-xs" :title="item.name">
                {{ item.name }}
              </td>
              <td class="px-3 py-1.5 text-right tabular-nums">{{ formatNumber(item.count) }}</td>
              <td
                class="px-3 py-1.5 text-right tabular-nums"
                :class="item.flagged > 0 && kind !== 'cache' && 'text-destructive'"
              >
                {{ share(item.flagged, item.count) }}
              </td>
              <td class="px-3 py-1.5 text-right tabular-nums">{{ formatDuration(item.p50_ms) }}</td>
              <td class="px-3 py-1.5 text-right tabular-nums">{{ formatDuration(item.p95_ms) }}</td>
              <td class="hidden px-3 py-1.5 text-right tabular-nums sm:table-cell">
                {{ formatDuration(item.p99_ms) }}
              </td>
              <td class="hidden px-3 py-1.5 text-right tabular-nums md:table-cell">
                {{ formatDuration(item.total_ms) }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <aside
      v-if="selected"
      class="grid content-start gap-4 border-l bg-card p-4"
      :aria-label="t('performance.detail')"
    >
      <div class="flex items-start justify-between gap-2">
        <h2 class="font-mono text-sm break-all">{{ selected }}</h2>
        <button
          type="button"
          class="text-muted-foreground hover:text-foreground"
          :aria-label="t('common.close')"
          @click="update({ name: undefined })"
        >
          <X class="size-4" />
        </button>
      </div>
      <dl v-if="row" class="grid grid-cols-3 gap-3 text-sm">
        <div>
          <dt class="text-xs text-muted-foreground">{{ t('performance.count') }}</dt>
          <dd class="tabular-nums">{{ formatNumber(row.count) }}</dd>
        </div>
        <div>
          <dt class="text-xs text-muted-foreground">{{ t('performance.average') }}</dt>
          <dd class="tabular-nums">{{ formatDuration(row.average_ms) }}</dd>
        </div>
        <div>
          <dt class="text-xs text-muted-foreground">{{ t('performance.max') }}</dt>
          <dd class="tabular-nums">{{ formatDuration(row.max_ms) }}</dd>
        </div>
      </dl>
      <template v-if="series.data.value">
        <SeriesChart
          :points="countPoints"
          :title="t('performance.chart.count')"
          :label="t('performance.count')"
          :part-label="flaggedLabel"
          :format="formatNumber"
        />
        <SeriesChart
          :points="p95Points"
          :title="t('performance.chart.p95')"
          label="p95"
          :format="formatDuration"
        />
      </template>
      <section v-if="transactions" class="grid gap-2">
        <h3 class="text-sm font-medium">{{ t('performance.slowest') }}</h3>
        <p v-if="samples.data.value?.length === 0" class="text-xs text-muted-foreground">
          {{ t('performance.samplesGone') }}
        </p>
        <ol class="divide-y rounded-md border text-xs">
          <li v-for="sample in samples.data.value ?? []" :key="sample.trace_id + sample.start">
            <RouterLink
              :to="`/traces/${sample.trace_id}`"
              class="grid grid-cols-[5rem_minmax(0,1fr)_auto] items-center gap-2 px-3 py-1.5 hover:bg-accent"
            >
              <span class="font-medium tabular-nums">{{ formatDuration(sample.duration_ms) }}</span>
              <span class="truncate text-muted-foreground" :title="formatDateTime(sample.start)">
                {{ formatRelative(sample.start) }}
                <template v-if="scope.project === null">
                  · {{ names.project(sample.project_id) }}</template
                >
                <template v-if="sample.service"> · {{ sample.service }}</template>
              </span>
              <Badge v-if="sample.failed" variant="destructive">{{
                sample.http_status ?? t('performance.failed')
              }}</Badge>
              <span v-else class="text-muted-foreground tabular-nums">{{
                sample.http_status ?? ''
              }}</span>
            </RouterLink>
          </li>
        </ol>
      </section>
    </aside>
  </div>
</template>
