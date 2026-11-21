<script setup lang="ts">
import { computed, ref } from 'vue'

import { formatDateTime, formatTick } from '@/shared/lib/format'

export interface Point {
  start: string
  value: number
  /** The flagged share of `value` (failures, misses), stacked on top. */
  part?: number
}

const props = defineProps<{
  points: Point[]
  title: string
  label: string
  /** Names the stacked share; without it the chart has one series. */
  partLabel?: string
  format: (value: number) => string
}>()

const WIDTH = 600
const HEIGHT = 96
const GAP = 2

const max = computed(() => Math.max(1e-9, ...props.points.map((p) => p.value)))
const peak = computed(() => Math.max(0, ...props.points.map((p) => p.value)))
const slot = computed(() => WIDTH / Math.max(1, props.points.length))
const total = computed(() => props.points.reduce((sum, p) => sum + p.value, 0))
const partTotal = computed(() => props.points.reduce((sum, p) => sum + (p.part ?? 0), 0))

/** The first, middle and last bucket's start, under the bars. */
const ticks = computed(() => {
  const points = props.points
  const first = points[0]
  const last = points[points.length - 1]
  if (!first || !last || points.length < 2) return []
  const span = new Date(last.start).getTime() - new Date(first.start).getTime()
  const middle = points[Math.floor(points.length / 2)]!
  return [first, middle, last].map((p, i) => ({
    key: `${i}-${p.start}`,
    text: formatTick(p.start, span),
    align: ['text-left', 'text-center', 'text-right'][i],
  }))
})

function bar(point: Point) {
  const height = point.value === 0 ? 0 : Math.max(2, (point.value / max.value) * (HEIGHT - 6))
  const part = point.part ? Math.min(height, Math.max(2, (point.part / point.value) * height)) : 0
  return { height, part }
}

const hovered = ref<number | null>(null)
const tip = computed(() => {
  if (hovered.value === null) return null
  const point = props.points[hovered.value]
  return point ? { ...point, left: ((hovered.value + 0.5) * slot.value) / WIDTH } : null
})
</script>

<!-- One measure over time, its flagged share stacked on top; a bar's tooltip tells the numbers. -->
<template>
  <figure class="grid gap-3 rounded-xl bg-card p-4 shadow-panel">
    <figcaption class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      <span class="mr-auto text-sm font-medium text-foreground">{{ title }}</span>
      <template v-if="partLabel">
        <span class="inline-flex items-center gap-1.5">
          <span class="size-2 rounded-full bg-level-info" aria-hidden="true" />{{ label }}
        </span>
        <span class="inline-flex items-center gap-1.5">
          <span class="size-2 rounded-full bg-level-error" aria-hidden="true" />{{ partLabel }}
          <span class="font-medium text-foreground tabular-nums">{{ format(partTotal) }}</span>
        </span>
      </template>
    </figcaption>
    <div class="relative" @mouseleave="hovered = null">
      <span
        v-if="peak > 0"
        class="pointer-events-none absolute -top-1.5 left-0 z-[1] rounded bg-card pr-1 text-[10px] text-muted-foreground tabular-nums"
        >{{ format(peak) }}</span
      >
      <svg
        :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
        preserveAspectRatio="none"
        class="h-24 w-full overflow-visible"
        role="img"
        :aria-label="`${title}: ${format(total)}`"
      >
        <line
          v-for="y in [6, HEIGHT / 2 + 3]"
          :key="y"
          :x1="0"
          :x2="WIDTH"
          :y1="y"
          :y2="y"
          stroke="var(--border)"
          stroke-dasharray="3 4"
          vector-effect="non-scaling-stroke"
        />
        <line
          :x1="0"
          :x2="WIDTH"
          :y1="HEIGHT - 0.5"
          :y2="HEIGHT - 0.5"
          stroke="var(--border)"
          vector-effect="non-scaling-stroke"
        />
        <rect
          v-if="tip"
          :x="hovered! * slot"
          y="0"
          :width="slot"
          :height="HEIGHT"
          fill="var(--foreground)"
          opacity="0.05"
        />
        <g v-for="(point, index) in points" :key="point.start">
          <template v-if="point.value > 0">
            <rect
              :x="index * slot + GAP / 2"
              :y="HEIGHT - bar(point).height"
              :width="Math.max(1, slot - GAP)"
              :height="
                Math.max(0, bar(point).height - (bar(point).part ? bar(point).part + GAP : 0))
              "
              fill="var(--level-info)"
              :opacity="hovered === null || hovered === index ? 1 : 0.6"
              rx="1.5"
            />
            <rect
              v-if="bar(point).part"
              :x="index * slot + GAP / 2"
              :y="HEIGHT - bar(point).height"
              :width="Math.max(1, slot - GAP)"
              :height="bar(point).part"
              fill="var(--level-error)"
              :opacity="hovered === null || hovered === index ? 1 : 0.6"
              rx="1.5"
            />
          </template>
          <rect
            :x="index * slot"
            y="0"
            :width="slot"
            :height="HEIGHT"
            fill="transparent"
            data-point
            @mouseenter="hovered = index"
          />
        </g>
      </svg>
      <div
        v-if="ticks.length"
        class="mt-1.5 grid grid-cols-3 text-[10px] text-muted-foreground tabular-nums"
        aria-hidden="true"
      >
        <span v-for="tick in ticks" :key="tick.key" :class="tick.align">{{ tick.text }}</span>
      </div>
      <div
        v-if="tip"
        class="pointer-events-none absolute top-0 z-10 -translate-x-1/2 -translate-y-2 animate-fade-in rounded-lg bg-popover px-3 py-2 text-xs shadow-pop"
        :style="{ left: `${Math.min(85, Math.max(15, tip.left * 100))}%` }"
        role="tooltip"
      >
        <p class="mb-1 font-medium">{{ formatDateTime(tip.start) }}</p>
        <p class="flex items-center justify-between gap-4">
          <span class="inline-flex items-center gap-1.5 text-muted-foreground"
            ><span class="size-2 rounded-full bg-level-info" aria-hidden="true" />{{ label }}</span
          ><span class="font-medium tabular-nums">{{ format(tip.value) }}</span>
        </p>
        <p v-if="partLabel" class="flex items-center justify-between gap-4">
          <span class="inline-flex items-center gap-1.5 text-muted-foreground"
            ><span class="size-2 rounded-full bg-level-error" aria-hidden="true" />{{
              partLabel
            }}</span
          ><span class="font-medium tabular-nums">{{ format(tip.part ?? 0) }}</span>
        </p>
      </div>
    </div>
  </figure>
</template>
