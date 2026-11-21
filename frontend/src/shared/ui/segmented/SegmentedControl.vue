<script setup lang="ts" generic="T extends string">
import { useId } from 'vue'

import { cn } from '@/shared/lib/utils'

/** One of a few options, all in view: a time range, a kind. */
const props = defineProps<{
  options: readonly { value: T; label: string; title?: string }[]
  label: string
  class?: string
}>()
const value = defineModel<T>({ required: true })
const name = useId()

function step(from: number, by: number): void {
  const options = props.options
  const next = options[(from + by + options.length) % options.length]
  if (next) value.value = next.value
}
</script>

<!--
  A radio group drawn as one control: arrow keys move between the options,
  as the platform's own radio buttons do.
-->
<template>
  <div
    role="radiogroup"
    :aria-label="label"
    :class="
      cn(
        'inline-flex h-8 max-w-full items-center gap-0.5 overflow-x-auto rounded-lg border border-input bg-muted/60 p-0.5 dark:bg-input/20',
        props.class,
      )
    "
  >
    <button
      v-for="(option, index) in options"
      :key="option.value"
      type="button"
      role="radio"
      :name="name"
      :aria-checked="option.value === value"
      :tabindex="option.value === value ? 0 : -1"
      :title="option.title"
      class="h-full shrink-0 cursor-pointer rounded-md px-2.5 text-xs font-medium whitespace-nowrap text-muted-foreground transition-colors outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60 aria-checked:bg-card aria-checked:text-foreground aria-checked:shadow-xs dark:aria-checked:bg-input/60"
      @click="value = option.value"
      @keydown.right.prevent="step(index, 1)"
      @keydown.down.prevent="step(index, 1)"
      @keydown.left.prevent="step(index, -1)"
      @keydown.up.prevent="step(index, -1)"
    >
      {{ option.label }}
    </button>
  </div>
</template>
