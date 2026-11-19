<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { components } from '@/shared/api/schema'
import { formatDateTime, formatNumber } from '@/shared/lib/format'
import { levelGroup } from '@/shared/ui/level'

const props = defineProps<{ hours: components['schemas']['HourCount'][]; level: string }>()
const { t } = useI18n()

/** Hours per bar: 14 days make 84 bars, wide enough to point at. */
const PER_BAR = 4
const WIDTH = 1000
const HEIGHT = 80
const GAP = 2

const bars = computed(() => {
  const result: { start: string; count: number }[] = []
  for (let i = 0; i < props.hours.length; i += PER_BAR) {
    const group = props.hours.slice(i, i + PER_BAR)
    result.push({ start: group[0]!.hour, count: group.reduce((sum, h) => sum + h.count, 0) })
  }
  return result
})
const total = computed(() => bars.value.reduce((sum, b) => sum + b.count, 0))
const max = computed(() => Math.max(1, ...bars.value.map((b) => b.count)))
const slot = computed(() => WIDTH / Math.max(1, bars.value.length))
const events = (n: number) => t('issues.eventCount', { n: formatNumber(n) }, n)
const fill = computed(() => `var(--level-${levelGroup(props.level)})`)

function height(count: number): number {
  return count === 0 ? 0 : Math.max(2, (count / max.value) * (HEIGHT - 4))
}

const hovered = ref<number | null>(null)
const tip = computed(() => {
  if (hovered.value === null) return null
  const bar = bars.value[hovered.value]
  if (!bar) return null
  const end = new Date(new Date(bar.start).getTime() + PER_BAR * 3600 * 1000).toISOString()
  return { ...bar, end, left: ((hovered.value + 0.5) * slot.value) / WIDTH }
})
</script>

<!-- The issue's events over the last 14 days, four hours per bar. -->
<template>
  <figure class="grid gap-2">
    <figcaption class="flex items-baseline justify-between gap-2 text-xs text-muted-foreground">
      <span class="font-medium text-foreground">{{ t('issues.chart.title') }}</span>
      <span class="tabular-nums">{{ events(total) }}</span>
    </figcaption>
    <div class="relative" @mouseleave="hovered = null">
      <svg
        :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
        preserveAspectRatio="none"
        class="h-20 w-full"
        role="img"
        :aria-label="t('issues.chart.label', { events: events(total) })"
      >
        <line :x1="0" :x2="WIDTH" :y1="HEIGHT - 0.5" :y2="HEIGHT - 0.5" stroke="var(--border)" />
        <g v-for="(bar, index) in bars" :key="bar.start">
          <rect
            v-if="bar.count > 0"
            :x="index * slot"
            :y="HEIGHT - height(bar.count)"
            :width="Math.max(1, slot - GAP)"
            :height="height(bar.count)"
            :fill="fill"
            :opacity="hovered === null || hovered === index ? 1 : 0.55"
            rx="1"
          />
          <rect
            :x="index * slot"
            y="0"
            :width="slot"
            :height="HEIGHT"
            fill="transparent"
            data-bar
            @mouseenter="hovered = index"
          />
        </g>
      </svg>
      <div
        v-if="tip"
        class="pointer-events-none absolute top-0 z-10 -translate-x-1/2 rounded-md border bg-popover px-3 py-2 text-xs shadow-md"
        :style="{ left: `${Math.min(88, Math.max(12, tip.left * 100))}%` }"
        role="tooltip"
      >
        <p class="font-medium">{{ formatDateTime(tip.start) }} – {{ formatDateTime(tip.end) }}</p>
        <p class="tabular-nums">{{ events(tip.count) }}</p>
      </div>
    </div>
    <div class="flex justify-between text-[11px] text-muted-foreground" aria-hidden="true">
      <span>{{ t('issues.chart.daysAgo', { days: 14 }) }}</span>
      <span>{{ t('issues.chart.now') }}</span>
    </div>
  </figure>
</template>
