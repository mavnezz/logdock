<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { type LocationQuery, RouterLink, useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDuration, formatNumber, formatRelative } from '@/shared/lib/format'
import { useProjectNames } from '@/shared/scope/projects'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { LevelBadge } from '@/shared/ui/level'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, PageHeader } from '@/shared/ui/page'
import { type Point, SeriesChart } from '@/shared/ui/series-chart'

import { type OverviewRange, overviewParams, RANGES, useOverview } from './api'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const scope = useScopeStore()
const names = useProjectNames()

function one(query: LocationQuery, key: string): string {
  const value = query[key]
  return (Array.isArray(value) ? value[0] : value) ?? ''
}
const range = computed<OverviewRange>(() => {
  const value = one(route.query, 'range')
  return (RANGES as readonly string[]).includes(value) ? (value as OverviewRange) : '24h'
})
function choose(next: string): void {
  void router.replace({ query: next === '24h' ? {} : { range: next } })
}

// The range ends now, and moves on every minute while the page is open.
const now = ref(new Date())
watch([range, () => scope.project, () => scope.environment], () => (now.value = new Date()))
let timer: ReturnType<typeof setInterval> | undefined
onMounted(() => (timer = setInterval(() => (now.value = new Date()), 60_000)))
onBeforeUnmount(() => clearInterval(timer))

const params = computed(() =>
  overviewParams(
    { range: range.value, project: scope.project, environment: scope.environment },
    now.value,
  ),
)
const overview = useOverview(params)
const data = computed(() => overview.data.value)
const several = computed(() => scope.project === null)

const share = (part: number, of: number) =>
  of > 0 ? `${(Math.round((part / of) * 1000) / 10).toLocaleString()} %` : '–'

const requests = computed(() => data.value?.requests.total)
const jobs = computed(() => data.value?.jobs.total)
const errors = computed(() => data.value?.errors)
const logs = computed(() => data.value?.logs.total)
const monitors = computed(() => data.value?.monitors)
const logErrors = computed(() => (logs.value ? logs.value.error + logs.value.fatal : 0))

const empty = computed(
  () =>
    !!data.value &&
    requests.value?.count === 0 &&
    jobs.value?.count === 0 &&
    errors.value?.events === 0 &&
    monitors.value?.total === 0 &&
    Object.values(logs.value ?? {}).every((n) => n === 0),
)

const requestPoints = computed<Point[]>(
  () =>
    data.value?.requests.buckets.map((b) => ({
      start: b.start,
      value: b.count,
      part: b.failed,
    })) ?? [],
)
const p95Points = computed<Point[]>(
  () => data.value?.requests.buckets.map((b) => ({ start: b.start, value: b.p95_ms })) ?? [],
)
const jobPoints = computed<Point[]>(
  () =>
    data.value?.jobs.buckets.map((b) => ({ start: b.start, value: b.count, part: b.failed })) ?? [],
)
const errorPoints = computed<Point[]>(
  () => data.value?.errors.buckets.map((b) => ({ start: b.start, value: b.count })) ?? [],
)
const logPoints = computed<Point[]>(
  () =>
    data.value?.logs.buckets.map((b) => ({
      start: b.start,
      value: b.trace + b.debug + b.info + b.warn + b.error + b.fatal,
      part: b.error + b.fatal,
    })) ?? [],
)

/** The log page, at the errors of the same range. */
const logsLink = computed(() => ({
  path: '/logs',
  query: { level: 'error,fatal', range: range.value },
}))
const routeLink = (name: string) => ({
  path: '/performance',
  query: { name, ...(range.value === '24h' ? {} : { range: range.value }) },
})
const isNew = (firstSeen: string) => !!data.value && firstSeen >= data.value.from
</script>

<template>
  <div class="grid content-start gap-6">
    <PageHeader :title="t('overview.title')" :description="t('overview.description')">
      <NativeSelect
        :model-value="range"
        class="w-44"
        :aria-label="t('overview.range')"
        @update:model-value="(value) => choose(String(value))"
      >
        <option v-for="option in RANGES" :key="option" :value="option">
          {{ t(`logs.ranges.${option}`) }}
        </option>
      </NativeSelect>
      <ScopePicker />
    </PageHeader>

    <Alert v-if="overview.error.value" variant="destructive">{{
      problemMessage(overview.error.value)
    }}</Alert>
    <p v-else-if="!data" class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    <EmptyState
      v-else-if="empty"
      :title="t('overview.empty')"
      :description="t('overview.emptyHint')"
    />
    <template v-else-if="data && requests && jobs && errors && logs && monitors">
      <dl
        class="grid grid-cols-2 gap-px overflow-hidden rounded-lg border bg-border sm:grid-cols-3 xl:grid-cols-6"
      >
        <RouterLink to="/performance" class="grid gap-1 bg-card p-4 hover:bg-accent/50">
          <dt class="text-xs text-muted-foreground">{{ t('overview.requests') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">{{ formatNumber(requests.count) }}</dd>
          <dd
            class="text-xs"
            :class="requests.flagged ? 'text-destructive' : 'text-muted-foreground'"
          >
            {{ t('overview.failedShare', { share: share(requests.flagged, requests.count) }) }}
          </dd>
        </RouterLink>
        <RouterLink to="/performance" class="grid gap-1 bg-card p-4 hover:bg-accent/50">
          <dt class="text-xs text-muted-foreground">{{ t('overview.p95') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">
            {{ requests.count ? formatDuration(requests.p95_ms) : '–' }}
          </dd>
          <dd class="text-xs text-muted-foreground">
            {{
              requests.count
                ? t('overview.average', { time: formatDuration(requests.average_ms) })
                : t('overview.noRequests')
            }}
          </dd>
        </RouterLink>
        <RouterLink
          :to="{ path: '/performance', query: { kind: 'job' } }"
          class="grid gap-1 bg-card p-4 hover:bg-accent/50"
        >
          <dt class="text-xs text-muted-foreground">{{ t('overview.jobs') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">{{ formatNumber(jobs.count) }}</dd>
          <dd class="text-xs" :class="jobs.flagged ? 'text-destructive' : 'text-muted-foreground'">
            {{ t('overview.failed', { n: formatNumber(jobs.flagged) }) }}
          </dd>
        </RouterLink>
        <RouterLink to="/issues" class="grid gap-1 bg-card p-4 hover:bg-accent/50">
          <dt class="text-xs text-muted-foreground">{{ t('overview.errors') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">{{ formatNumber(errors.events) }}</dd>
          <dd class="text-xs text-muted-foreground">
            {{ t('overview.newIssues', { n: formatNumber(errors.new_issues) }, errors.new_issues)
            }}<template v-if="errors.regressed">
              · {{ t('overview.regressed', { n: formatNumber(errors.regressed) }) }}</template
            >
          </dd>
        </RouterLink>
        <RouterLink :to="logsLink" class="grid gap-1 bg-card p-4 hover:bg-accent/50">
          <dt class="text-xs text-muted-foreground">{{ t('overview.logErrors') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">{{ formatNumber(logErrors) }}</dd>
          <dd class="text-xs text-muted-foreground">
            {{ t('overview.warnings', { n: formatNumber(logs.warn) }) }}
          </dd>
        </RouterLink>
        <RouterLink to="/monitors" class="grid gap-1 bg-card p-4 hover:bg-accent/50">
          <dt class="text-xs text-muted-foreground">{{ t('overview.monitors') }}</dt>
          <dd class="text-2xl font-semibold tabular-nums">
            {{ monitors.total ? `${monitors.ok} / ${monitors.total}` : '–' }}
          </dd>
          <dd
            class="text-xs"
            :class="monitors.failing.length ? 'text-destructive' : 'text-muted-foreground'"
          >
            {{
              monitors.total === 0
                ? t('overview.noMonitors')
                : monitors.failing.length
                  ? t('overview.monitorsFailing', { n: monitors.failing.length })
                  : t('overview.monitorsOk')
            }}
          </dd>
        </RouterLink>
      </dl>

      <div class="grid gap-6 lg:grid-cols-2">
        <SeriesChart
          :points="requestPoints"
          :title="t('overview.requests')"
          :label="t('overview.requests')"
          :part-label="t('performance.failures')"
          :format="formatNumber"
        />
        <SeriesChart
          :points="p95Points"
          :title="t('overview.p95')"
          label="p95"
          :format="formatDuration"
        />
        <SeriesChart
          :points="jobPoints"
          :title="t('overview.jobs')"
          :label="t('overview.jobs')"
          :part-label="t('performance.failures')"
          :format="formatNumber"
        />
        <SeriesChart
          :points="errorPoints"
          :title="t('overview.errorsPerHour')"
          :label="t('overview.errors')"
          :format="formatNumber"
        />
        <SeriesChart
          class="lg:col-span-2"
          :points="logPoints"
          :title="t('overview.logs')"
          :label="t('overview.logEntries')"
          :part-label="t('overview.logErrors')"
          :format="formatNumber"
        />
      </div>

      <div class="grid gap-6 lg:grid-cols-2">
        <section class="grid content-start gap-2">
          <h2 class="text-sm font-medium">{{ t('overview.topIssues') }}</h2>
          <p v-if="errors.top.length === 0" class="text-sm text-muted-foreground">
            {{ t('overview.noErrors') }}
          </p>
          <ol v-else class="divide-y rounded-lg border" :aria-label="t('overview.topIssues')">
            <li v-for="issue in errors.top" :key="issue.id" class="grid gap-1 px-3 py-2">
              <div class="flex min-w-0 items-center gap-2">
                <LevelBadge :level="issue.level" />
                <RouterLink
                  :to="`/issues/${issue.id}`"
                  class="truncate text-sm font-medium hover:underline"
                  :title="issue.title"
                  >{{ issue.title }}</RouterLink
                >
              </div>
              <p class="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
                <span class="text-foreground tabular-nums">{{
                  t('issues.eventCount', { n: formatNumber(issue.events) }, issue.events)
                }}</span>
                <span v-if="issue.users">{{
                  t('issues.userCount', { n: formatNumber(issue.users) }, issue.users)
                }}</span>
                <span v-if="several">{{ names.project(issue.project_id) }}</span>
                <span>{{ names.environment(issue.environment_id) }}</span>
                <span>{{ formatRelative(issue.last_seen) }}</span>
                <Badge v-if="isNew(issue.first_seen)" variant="warning">{{
                  t('overview.new')
                }}</Badge>
              </p>
            </li>
          </ol>
        </section>

        <section class="grid content-start gap-2">
          <h2 class="text-sm font-medium">{{ t('overview.slowestRoutes') }}</h2>
          <p v-if="data.slowest_routes.length === 0" class="text-sm text-muted-foreground">
            {{ t('overview.noRequests') }}
          </p>
          <ol v-else class="divide-y rounded-lg border" :aria-label="t('overview.slowestRoutes')">
            <li v-for="row in data.slowest_routes" :key="row.name">
              <RouterLink
                :to="routeLink(row.name)"
                class="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 px-3 py-2 text-sm hover:bg-accent/50"
              >
                <span class="truncate font-mono text-xs" :title="row.name">{{ row.name }}</span>
                <span class="text-xs text-muted-foreground tabular-nums">{{
                  formatNumber(row.count)
                }}</span>
                <span class="w-16 text-right tabular-nums">{{ formatDuration(row.p95_ms) }}</span>
              </RouterLink>
            </li>
          </ol>

          <template v-if="data.failing_routes.length">
            <h2 class="mt-4 text-sm font-medium">{{ t('overview.failingRoutes') }}</h2>
            <ol class="divide-y rounded-lg border" :aria-label="t('overview.failingRoutes')">
              <li v-for="row in data.failing_routes" :key="row.name">
                <RouterLink
                  :to="routeLink(row.name)"
                  class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-sm hover:bg-accent/50"
                >
                  <span class="truncate font-mono text-xs" :title="row.name">{{ row.name }}</span>
                  <span class="text-destructive tabular-nums">{{
                    t('overview.failedOf', {
                      n: formatNumber(row.flagged),
                      share: share(row.flagged, row.count),
                    })
                  }}</span>
                </RouterLink>
              </li>
            </ol>
          </template>

          <template v-if="monitors.failing.length">
            <h2 class="mt-4 text-sm font-medium">{{ t('overview.failingMonitors') }}</h2>
            <ul class="divide-y rounded-lg border" :aria-label="t('overview.failingMonitors')">
              <li v-for="monitor in monitors.failing" :key="monitor.id">
                <RouterLink
                  :to="`/monitors/${monitor.id}`"
                  class="flex items-center justify-between gap-3 px-3 py-2 text-sm hover:bg-accent/50"
                >
                  <span class="truncate">{{ monitor.name }}</span>
                  <Badge variant="destructive">{{ t(`monitors.health.${monitor.health}`) }}</Badge>
                </RouterLink>
              </li>
            </ul>
          </template>
        </section>
      </div>
    </template>
  </div>
</template>
