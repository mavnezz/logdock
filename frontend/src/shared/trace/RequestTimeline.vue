<script setup lang="ts">
import { ChevronDown, ChevronRight, ZoomIn, ZoomOut } from 'lucide-vue-next'
import { computed, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatDuration } from '@/shared/lib/format'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'

import {
  breakdown,
  buildTimeline,
  type Category,
  type Node,
  type TraceTransaction,
  transactionOf,
} from './timeline'

const props = defineProps<{
  transactions: TraceTransaction[]
  /** The span an error happened in: its transaction is opened and marked. */
  focusSpan?: string | null
  /** The transaction to open when no span is given, by its name. */
  focusName?: string | null
}>()
const { t } = useI18n()

const timeline = computed(() => buildTimeline(props.transactions))
const focus = computed(() => transactionOf(timeline.value, props.focusSpan, props.focusName))

/** Bars are coloured by what the time went to. */
const COLORS: Record<Category, string> = {
  db: 'var(--chart-1)',
  http: 'var(--chart-2)',
  cache: 'var(--chart-3)',
  view: 'var(--chart-4)',
  queue: 'var(--chart-5)',
  framework: 'color-mix(in oklab, var(--muted-foreground) 45%, transparent)',
  app: 'color-mix(in oklab, var(--foreground) 55%, transparent)',
}

const collapsed = ref(new Set<string>())
// Shallow: compared by identity with the rows' nodes, not wrapped.
const selected = shallowRef<Node | null>(null)
/** The part shown, in microseconds of the timeline; `null`: all of it. */
const zoom = ref<{ from: number; to: number } | null>(null)

// Repeated spans start folded; so does every transaction but the focused one
// when there are several.
watch(
  timeline,
  (value) => {
    const folded = new Set<string>()
    let transactions = 0
    const count = (n: Node) => {
      if (n.kind === 'transaction') transactions++
      n.children.forEach(count)
    }
    value.roots.forEach(count)
    const several = transactions > 1
    const visit = (n: Node) => {
      if (n.kind === 'group') folded.add(n.key)
      if (several && n.kind === 'transaction' && n !== focus.value && !contains(n, focus.value)) {
        folded.add(n.key)
      }
      n.children.forEach(visit)
    }
    value.roots.forEach(visit)
    collapsed.value = folded
    selected.value = null
    zoom.value = null
  },
  { immediate: true },
)

function contains(n: Node, wanted: Node | null): boolean {
  return wanted !== null && n.children.some((c) => c === wanted || contains(c, wanted))
}

interface Row {
  node: Node
  depth: number
}
const rows = computed(() => {
  const out: Row[] = []
  const visit = (n: Node, depth: number) => {
    out.push({ node: n, depth })
    if (!collapsed.value.has(n.key)) n.children.forEach((c) => visit(c, depth + 1))
  }
  timeline.value.roots.forEach((r) => visit(r, 0))
  return out
})

function toggle(n: Node): void {
  const next = new Set(collapsed.value)
  if (next.has(n.key)) next.delete(n.key)
  else next.add(n.key)
  collapsed.value = next
}

const view = computed(() => zoom.value ?? { from: 0, to: Math.max(1, timeline.value.total) })
function bar(n: Node): { left: string; width: string } | null {
  const { from, to } = view.value
  const span = to - from
  const start = Math.max(n.start, from)
  const end = Math.min(n.start + n.duration, to)
  if (end < start || n.start > to) return null
  return {
    left: `${((start - from) / span) * 100}%`,
    width: `max(2px, ${((end - start) / span) * 100}%)`,
  }
}

const parts = computed(() => (focus.value ? breakdown(focus.value) : []))
const partsTotal = computed(() => parts.value.reduce((sum, p) => sum + p.micros, 0) || 1)
const ms = (micros: number) => formatDuration(micros / 1000)
const share = (micros: number) => `${Math.round((micros / partsTotal.value) * 100)} %`

const details = computed(() => {
  const n = selected.value
  if (!n) return []
  const owner = transactionOf(timeline.value, n.spanId, null)
  const entries: [string, string][] = [
    [t('timeline.detail.op'), n.op],
    [t('timeline.detail.category'), t(`timeline.categories.${n.category}`)],
    [t('timeline.detail.start'), `+${ms(n.start - (owner?.start ?? 0))}`],
    [t('timeline.detail.duration'), ms(n.duration)],
    [t('timeline.detail.self'), ms(n.self)],
  ]
  if (n.kind === 'group') {
    entries.push([t('timeline.detail.repeated'), t('timeline.times', { count: n.count })])
    entries.push([t('timeline.detail.average'), ms(n.self / n.count)])
  }
  if (n.status) entries.push([t('timeline.detail.status'), n.status])
  for (const [key, value] of Object.entries(n.data)) {
    entries.push([key, typeof value === 'string' ? value : JSON.stringify(value)])
  }
  return entries
})
</script>

<template>
  <section class="grid gap-3">
    <div v-if="focus && parts.length" class="grid gap-2">
      <p class="text-sm">
        <span class="font-medium">{{ focus.label }}</span>
        <span class="text-muted-foreground"> · {{ ms(focus.duration) }}</span>
      </p>
      <div
        class="flex h-3 overflow-hidden rounded-sm bg-muted"
        role="img"
        :aria-label="t('timeline.breakdown')"
      >
        <span
          v-for="part in parts"
          :key="part.category"
          class="h-full border-r-2 border-background last:border-r-0"
          :style="{ width: share(part.micros), background: COLORS[part.category] }"
        />
      </div>
      <ul class="flex flex-wrap gap-x-4 gap-y-1 text-xs" :aria-label="t('timeline.breakdown')">
        <li v-for="part in parts" :key="part.category" class="flex items-center gap-1.5">
          <span class="size-2.5 rounded-sm" :style="{ background: COLORS[part.category] }" />
          <span>{{ t(`timeline.categories.${part.category}`) }}</span>
          <span class="text-muted-foreground tabular-nums"
            >{{ share(part.micros) }} · {{ ms(part.micros) }}</span
          >
        </li>
      </ul>
    </div>

    <div class="flex items-center justify-end gap-2">
      <Button
        v-if="selected && !zoom"
        size="sm"
        variant="outline"
        @click="
          zoom = { from: selected.start, to: selected.start + Math.max(1, selected.duration) }
        "
        ><ZoomIn />{{ t('timeline.zoomIn') }}</Button
      >
      <Button v-if="zoom" size="sm" variant="outline" @click="zoom = null"
        ><ZoomOut />{{ t('timeline.zoomOut') }}</Button
      >
    </div>

    <ol class="divide-y rounded-lg border text-xs" :aria-label="t('timeline.title')">
      <li
        v-for="row in rows"
        :key="row.node.key"
        class="grid cursor-pointer grid-cols-[minmax(0,2fr)_minmax(0,3fr)_4.5rem] items-center gap-3 px-3 py-1 hover:bg-accent/50"
        :class="[
          row.node.kind === 'transaction' && 'bg-muted/40 font-medium',
          selected === row.node && 'bg-accent',
        ]"
        @click="selected = selected === row.node ? null : row.node"
      >
        <span
          class="flex min-w-0 items-center gap-1.5"
          :style="{ paddingLeft: `${row.depth * 0.85}rem` }"
        >
          <button
            v-if="row.node.children.length"
            type="button"
            class="-m-1 shrink-0 rounded p-1 text-muted-foreground hover:text-foreground"
            :aria-label="
              collapsed.has(row.node.key) ? t('timeline.expand') : t('timeline.collapse')
            "
            :aria-expanded="!collapsed.has(row.node.key)"
            @click.stop="toggle(row.node)"
          >
            <ChevronRight v-if="collapsed.has(row.node.key)" class="size-3.5" />
            <ChevronDown v-else class="size-3.5" />
          </button>
          <span v-else class="w-3.5 shrink-0" />
          <span
            class="size-2 shrink-0 rounded-sm"
            :style="{
              background:
                row.node.kind === 'transaction' ? 'var(--level-info)' : COLORS[row.node.category],
            }"
          />
          <span class="shrink-0 text-muted-foreground">{{ row.node.op }}</span>
          <Badge v-if="row.node.kind === 'group'" variant="warning" class="shrink-0">{{
            t('timeline.times', { count: row.node.count })
          }}</Badge>
          <span class="truncate font-mono" :title="row.node.label">{{ row.node.label }}</span>
          <span v-if="row.node.detail" class="truncate text-muted-foreground">{{
            row.node.detail
          }}</span>
          <span
            v-if="focusSpan && row.node.spanId === focusSpan"
            class="shrink-0 font-medium text-destructive"
            >{{ t('timeline.errorHere') }}</span
          >
        </span>
        <span class="relative h-3" aria-hidden="true">
          <span
            v-if="bar(row.node)"
            class="absolute inset-y-0 rounded-sm"
            :class="row.node.failed && 'ring-2 ring-destructive'"
            :style="{
              ...bar(row.node),
              background:
                row.node.kind === 'transaction' ? 'var(--level-info)' : COLORS[row.node.category],
            }"
          />
        </span>
        <span class="text-right tabular-nums">{{ ms(row.node.duration) }}</span>
      </li>
    </ol>

    <dl
      v-if="selected"
      class="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1 rounded-lg border p-3 text-xs"
      :aria-label="t('timeline.detail.title')"
    >
      <dt class="col-span-2 mb-1 font-mono text-sm break-all whitespace-pre-wrap">
        {{ selected.label }}
      </dt>
      <template v-for="[key, value] in details" :key="key">
        <dt class="text-muted-foreground">{{ key }}</dt>
        <dd class="font-mono break-all">{{ value }}</dd>
      </template>
    </dl>
  </section>
</template>
