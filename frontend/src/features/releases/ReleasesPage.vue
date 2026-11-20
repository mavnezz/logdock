<script setup lang="ts">
import { ArrowDown, ArrowUp } from 'lucide-vue-next'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime, formatDuration, formatNumber, formatRelative } from '@/shared/lib/format'
import ScopePicker from '@/shared/scope/ScopePicker.vue'
import { useScopeStore } from '@/shared/scope/store'
import { Alert } from '@/shared/ui/alert'
import { EmptyState, PageHeader } from '@/shared/ui/page'

import { failureRate, type Release, useReleases } from './api'

const { t } = useI18n()
const scope = useScopeStore()
const releases = useReleases(
  computed(() => ({ project: scope.project, environment: scope.environment })),
)

interface Change {
  /** Up or down against the release before. */
  up: boolean
  /** Whether it got worse: a higher failure rate or p95. */
  worse: boolean
  text: string
}

/** How a value changed from the release before; nothing when either lacks it. */
function change(
  now: number | null | undefined,
  before: number | null | undefined,
  format: (n: number) => string,
): Change | null {
  if (
    now === null ||
    now === undefined ||
    before === null ||
    before === undefined ||
    now === before
  ) {
    return null
  }
  const up = now > before
  return { up, worse: up, text: `${up ? '+' : '−'}${format(Math.abs(now - before))}` }
}

const rows = computed(() => {
  const list = releases.data.value ?? []
  return list.map((release: Release, i: number) => {
    const before = list[i + 1]
    const rate = failureRate(release)
    return {
      release,
      rate,
      rateChange: before ? change(rate, failureRate(before), (n) => `${n.toFixed(1)} pp`) : null,
      p95Change: before
        ? change(release.requests?.p95_ms, before.requests?.p95_ms, (n) => formatDuration(n))
        : null,
    }
  })
})
const percent = (value: number | null) => (value === null ? '–' : `${value.toFixed(1)} %`)
</script>

<template>
  <div class="grid content-start gap-4">
    <PageHeader :title="t('releases.title')" :description="t('releases.description')">
      <ScopePicker />
    </PageHeader>

    <Alert v-if="releases.error.value" variant="destructive">{{
      problemMessage(releases.error.value)
    }}</Alert>
    <p v-if="releases.isPending.value" class="text-sm text-muted-foreground">
      {{ t('common.loading') }}
    </p>
    <EmptyState
      v-else-if="rows.length === 0"
      :title="t('releases.empty')"
      :description="t('releases.emptyHint')"
    />
    <div v-else class="overflow-x-auto rounded-lg border">
      <table class="w-full text-sm" :aria-label="t('releases.title')">
        <thead class="border-b bg-muted/40 text-xs text-muted-foreground">
          <tr>
            <th class="px-3 py-2 text-left font-medium">{{ t('releases.release') }}</th>
            <th class="hidden px-3 py-2 text-left font-medium sm:table-cell">
              {{ t('releases.firstSeen') }}
            </th>
            <th class="px-3 py-2 text-right font-medium">{{ t('releases.requests') }}</th>
            <th class="px-3 py-2 text-right font-medium">{{ t('releases.failed') }}</th>
            <th class="px-3 py-2 text-right font-medium">p95</th>
            <th class="px-3 py-2 text-right font-medium">{{ t('releases.errors') }}</th>
            <th class="px-3 py-2 text-right font-medium">{{ t('releases.newIssues') }}</th>
          </tr>
        </thead>
        <tbody class="divide-y">
          <tr v-for="row in rows" :key="row.release.release">
            <td class="px-3 py-1.5 font-mono text-xs break-all">{{ row.release.release }}</td>
            <td
              class="hidden px-3 py-1.5 text-muted-foreground sm:table-cell"
              :title="formatDateTime(row.release.first_seen)"
            >
              {{ formatRelative(row.release.first_seen) }}
            </td>
            <td class="px-3 py-1.5 text-right tabular-nums">
              {{ row.release.requests ? formatNumber(row.release.requests.count) : '–' }}
            </td>
            <td class="px-3 py-1.5 text-right tabular-nums">
              <span>{{ percent(row.rate) }}</span>
              <span
                v-if="row.rateChange"
                class="ml-1.5 inline-flex items-center gap-0.5 text-xs"
                :class="row.rateChange.worse ? 'text-destructive' : 'text-success'"
                :title="t('releases.againstBefore')"
                ><ArrowUp v-if="row.rateChange.up" class="size-3" /><ArrowDown
                  v-else
                  class="size-3"
                />{{ row.rateChange.text }}</span
              >
            </td>
            <td class="px-3 py-1.5 text-right tabular-nums">
              <span>{{
                row.release.requests ? formatDuration(row.release.requests.p95_ms) : '–'
              }}</span>
              <span
                v-if="row.p95Change"
                class="ml-1.5 inline-flex items-center gap-0.5 text-xs"
                :class="row.p95Change.worse ? 'text-destructive' : 'text-success'"
                :title="t('releases.againstBefore')"
                ><ArrowUp v-if="row.p95Change.up" class="size-3" /><ArrowDown
                  v-else
                  class="size-3"
                />{{ row.p95Change.text }}</span
              >
            </td>
            <td class="px-3 py-1.5 text-right tabular-nums">
              {{ formatNumber(row.release.events) }}
            </td>
            <td class="px-3 py-1.5 text-right tabular-nums">
              <RouterLink
                v-if="row.release.new_issues > 0"
                :to="{ path: '/issues', query: { status: 'all', release: row.release.release } }"
                class="font-medium text-destructive hover:underline"
                >{{ formatNumber(row.release.new_issues) }}</RouterLink
              >
              <span v-else class="text-muted-foreground">0</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
