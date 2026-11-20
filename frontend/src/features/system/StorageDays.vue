<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatBytes, formatDay } from '@/shared/lib/format'

import type { Storage } from './api'

const props = defineProps<{ segments: Storage['segments'] }>()
const { t } = useI18n()

/** Each kind's colour, in the charts' fixed order. */
const KINDS = [
  { kind: 'logs', color: 'var(--chart-1)' },
  { kind: 'events', color: 'var(--chart-2)' },
  { kind: 'traces', color: 'var(--chart-3)' },
] as const

const days = computed(() => {
  const byDay = new Map<string, Record<string, number>>()
  for (const s of props.segments) {
    const day = byDay.get(s.day) ?? { logs: 0, events: 0, traces: 0 }
    day[s.kind] = (day[s.kind] ?? 0) + s.bytes
    byDay.set(s.day, day)
  }
  return [...byDay.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, sizes]) => ({
      day,
      sizes,
      total: KINDS.reduce((sum, k) => sum + (sizes[k.kind] ?? 0), 0),
    }))
})
const max = computed(() => Math.max(1, ...days.value.map((d) => d.total)))

const WIDTH = 600
const HEIGHT = 120
const GAP = 2
const slot = computed(() => WIDTH / Math.max(1, days.value.length))

/** The stacked parts of a day's bar, from the bottom. */
function parts(sizes: Record<string, number>) {
  let y = HEIGHT
  return KINDS.filter((k) => (sizes[k.kind] ?? 0) > 0).map((k) => {
    const height = Math.max(1, ((sizes[k.kind] ?? 0) / max.value) * (HEIGHT - 4))
    y -= height
    return { ...k, y, height: Math.max(0, height - GAP) }
  })
}

const hovered = ref<number | null>(null)
const tip = computed(() => (hovered.value === null ? null : days.value[hovered.value]))
</script>

<!-- What each day of receipt takes, its kinds stacked; a bar's tooltip tells the sizes. -->
<template>
  <figure class="grid gap-1.5">
    <figcaption class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      <span class="font-medium text-foreground">{{ t('storage.perDay') }}</span>
      <span v-for="k in KINDS" :key="k.kind" class="inline-flex items-center gap-1.5">
        <span class="size-2.5 rounded-sm" :style="{ background: k.color }" aria-hidden="true" />{{
          t(`storage.kinds.${k.kind}`)
        }}
      </span>
    </figcaption>
    <div class="relative" @mouseleave="hovered = null">
      <svg
        :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
        preserveAspectRatio="none"
        class="h-32 w-full"
        role="img"
        :aria-label="t('storage.perDay')"
      >
        <line :x1="0" :x2="WIDTH" :y1="HEIGHT - 0.5" :y2="HEIGHT - 0.5" stroke="var(--border)" />
        <g
          v-for="(day, index) in days"
          :key="day.day"
          :opacity="hovered === null || hovered === index ? 1 : 0.55"
          data-day
          @mouseenter="hovered = index"
        >
          <rect :x="index * slot" y="0" :width="slot" :height="HEIGHT" fill="transparent" />
          <rect
            v-for="part in parts(day.sizes)"
            :key="part.kind"
            :x="index * slot"
            :y="part.y"
            :width="Math.max(1, slot - GAP)"
            :height="part.height"
            :fill="part.color"
            rx="1"
          />
        </g>
      </svg>
      <div
        v-if="tip"
        role="tooltip"
        class="pointer-events-none absolute -top-2 z-10 grid -translate-x-1/2 -translate-y-full gap-0.5 rounded-md border bg-popover px-2 py-1 text-xs shadow-md"
        :style="{ left: `${((((hovered ?? 0) + 0.5) * slot) / WIDTH) * 100}%` }"
      >
        <span class="font-medium">{{ formatDay(tip.day) }} · {{ formatBytes(tip.total) }}</span>
        <span v-for="k in KINDS" :key="k.kind" class="tabular-nums text-muted-foreground"
          >{{ t(`storage.kinds.${k.kind}`) }}: {{ formatBytes(tip.sizes[k.kind] ?? 0) }}</span
        >
      </div>
    </div>
  </figure>
</template>
