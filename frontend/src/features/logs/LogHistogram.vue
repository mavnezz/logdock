<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatDateTime, formatNumber } from '@/shared/lib/format'

import type { Histogram } from './api'
import type { LevelGroup } from './filters'

const props = defineProps<{ histogram: Histogram }>()
const emit = defineEmits<{ select: [range: { from: string; to: string }] }>()
const { t } = useI18n()

/** Bottom to top: what matters most sits on the baseline's far end, visible. */
const GROUPS: LevelGroup[] = ['debug', 'info', 'warn', 'error']
const FILL: Record<LevelGroup, string> = {
  debug: 'var(--level-debug)',
  info: 'var(--level-info)',
  warn: 'var(--level-warn)',
  error: 'var(--level-error)',
}

const WIDTH = 1000
const HEIGHT = 120
const GAP = 2

const buckets = computed(() =>
  props.histogram.buckets.map((b) => ({
    start: b.start,
    groups: {
      debug: b.trace + b.debug,
      info: b.info,
      warn: b.warn,
      error: b.error + b.fatal,
    },
    total: b.trace + b.debug + b.info + b.warn + b.error + b.fatal,
  })),
)
const max = computed(() => Math.max(1, ...buckets.value.map((b) => b.total)))
const totals = computed(() => {
  const sums: Record<LevelGroup, number> = { debug: 0, info: 0, warn: 0, error: 0 }
  for (const bucket of buckets.value) for (const g of GROUPS) sums[g] += bucket.groups[g]
  return sums
})
const slot = computed(() => WIDTH / Math.max(1, buckets.value.length))
const barWidth = computed(() => Math.max(1, slot.value - GAP))

/** The stacked segments of one bucket, with a 2 px gap between them. */
function segments(index: number) {
  const bucket = buckets.value[index]!
  let y = HEIGHT
  const parts: { group: LevelGroup; y: number; height: number }[] = []
  for (const group of GROUPS) {
    const value = bucket.groups[group]
    if (value === 0) continue
    const height = Math.max(1, (value / max.value) * (HEIGHT - 4))
    y -= height
    parts.push({ group, y, height: Math.max(1, height - (parts.length ? GAP : 0)) })
  }
  return parts
}

const hovered = ref<number | null>(null)
const tip = computed(() => {
  if (hovered.value === null) return null
  const bucket = buckets.value[hovered.value]
  return bucket ? { ...bucket, left: ((hovered.value + 0.5) * slot.value) / WIDTH } : null
})

function select(index: number): void {
  const bucket = buckets.value[index]
  if (!bucket) return
  const from = new Date(bucket.start)
  const to = new Date(from.getTime() + props.histogram.bucket_seconds * 1000)
  emit('select', { from: from.toISOString(), to: to.toISOString() })
}
</script>

<!-- Entries per level over time; a bar's tooltip says how many, a click narrows the range to it. -->
<template>
  <figure class="grid gap-2">
    <figcaption class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      <span class="font-medium text-foreground">{{ t('logs.chart.title') }}</span>
      <span v-for="group in GROUPS" :key="group" class="inline-flex items-center gap-1.5">
        <span class="size-2.5 rounded-sm" :style="{ background: FILL[group] }" aria-hidden="true" />
        {{ t(`logs.chart.groups.${group}`) }}
        <span class="tabular-nums text-foreground">{{ formatNumber(totals[group]) }}</span>
      </span>
    </figcaption>
    <div class="relative" @mouseleave="hovered = null">
      <svg
        :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
        preserveAspectRatio="none"
        class="h-28 w-full"
        role="img"
        :aria-label="t('logs.chart.label')"
      >
        <line :x1="0" :x2="WIDTH" :y1="HEIGHT - 0.5" :y2="HEIGHT - 0.5" stroke="var(--border)" />
        <g v-for="(bucket, index) in buckets" :key="bucket.start">
          <rect
            v-for="part in segments(index)"
            :key="part.group"
            :x="index * slot"
            :y="part.y"
            :width="barWidth"
            :height="part.height"
            :fill="FILL[part.group]"
            :opacity="hovered === null || hovered === index ? 1 : 0.55"
            rx="1"
          />
          <rect
            :x="index * slot"
            y="0"
            :width="slot"
            :height="HEIGHT"
            fill="transparent"
            class="cursor-pointer"
            data-bucket
            @mouseenter="hovered = index"
            @click="select(index)"
          />
        </g>
      </svg>
      <div
        v-if="tip"
        class="pointer-events-none absolute top-0 z-10 -translate-x-1/2 rounded-md border bg-popover px-3 py-2 text-xs shadow-md"
        :style="{ left: `${Math.min(90, Math.max(10, tip.left * 100))}%` }"
        role="tooltip"
      >
        <p class="mb-1 font-medium">{{ formatDateTime(new Date(tip.start).toISOString()) }}</p>
        <p v-for="group in GROUPS" :key="group" class="flex justify-between gap-4">
          <span>{{ t(`logs.chart.groups.${group}`) }}</span>
          <span class="tabular-nums">{{ formatNumber(tip.groups[group]) }}</span>
        </p>
      </div>
    </div>
  </figure>
</template>
