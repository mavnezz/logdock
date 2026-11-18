<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatDateTime, formatNumber } from '@/shared/lib/format'
import { Badge } from '@/shared/ui/badge'
import { EmptyState } from '@/shared/ui/page'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import { useIngestCounts } from './api'

const props = defineProps<{ projectId: number }>()
const i18n = useI18n()
const { t } = i18n
const counts = useIngestCounts(() => props.projectId)

/** One row per day: what all keys together brought. */
const days = computed(() => {
  const byDay = new Map<
    string,
    { accepted: number; skipped: number; rejected: Record<string, number> }
  >()
  for (const count of counts.data.value ?? []) {
    const day = byDay.get(count.day) ?? { accepted: 0, skipped: 0, rejected: {} }
    day.accepted += count.accepted
    day.skipped += count.skipped
    for (const [reason, n] of Object.entries(count.rejected)) {
      day.rejected[reason] = (day.rejected[reason] ?? 0) + (n ?? 0)
    }
    byDay.set(count.day, day)
  }
  return [...byDay.entries()].map(([day, c]) => ({ day, ...c }))
})

const last = computed(
  () =>
    (counts.data.value ?? [])
      .map((c) => c.last_rejection)
      .filter((r) => r != null)
      .sort((a, b) => b.at.localeCompare(a.at))[0],
)

const reason = (code: string) =>
  i18n.te(`ingest.reasons.${code}`) ? t(`ingest.reasons.${code}`) : code
</script>

<template>
  <section class="grid gap-3">
    <h2 class="font-medium">{{ t('ingest.title') }}</h2>
    <p class="text-sm text-muted-foreground">{{ t('ingest.description') }}</p>
    <div v-if="last" class="rounded-lg border border-warning/40 bg-warning/5 p-3 text-sm">
      <p class="font-medium">{{ t('ingest.lastRejection') }}: {{ reason(last.reason) }}</p>
      <p class="text-muted-foreground">{{ last.detail }} · {{ formatDateTime(last.at) }}</p>
    </div>
    <EmptyState
      v-if="days.length === 0"
      :title="t('ingest.empty')"
      :description="t('ingest.emptyHint')"
    />
    <Table v-else>
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('ingest.day') }}</TableHead>
          <TableHead class="text-right">{{ t('ingest.accepted') }}</TableHead>
          <TableHead class="text-right">{{ t('ingest.skipped') }}</TableHead>
          <TableHead>{{ t('ingest.rejected') }}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow v-for="day in days" :key="day.day">
          <TableCell class="font-mono text-xs">{{ day.day }}</TableCell>
          <TableCell class="text-right tabular-nums">{{ formatNumber(day.accepted) }}</TableCell>
          <TableCell class="text-right text-muted-foreground tabular-nums">{{
            formatNumber(day.skipped)
          }}</TableCell>
          <TableCell>
            <div class="flex flex-wrap gap-1">
              <Badge v-for="(n, code) in day.rejected" :key="code" variant="warning">
                {{ reason(String(code)) }}: {{ formatNumber(n) }}
              </Badge>
              <span v-if="Object.keys(day.rejected).length === 0" class="text-muted-foreground"
                >–</span
              >
            </div>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>
  </section>
</template>
