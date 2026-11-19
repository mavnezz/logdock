<script setup lang="ts">
import { computed } from 'vue'

import { levelGroup } from './filters'

const props = defineProps<{ level: string }>()
const group = computed(() => levelGroup(props.level))
const dot: Record<string, string> = {
  debug: 'bg-level-debug',
  info: 'bg-level-info',
  warn: 'bg-level-warn',
  error: 'bg-level-error',
}
</script>

<!-- A level as a coloured dot with its name: the colour is never alone. -->
<template>
  <span
    class="inline-flex items-center gap-1.5 font-mono text-xs uppercase"
    :class="level === 'fatal' && 'font-bold'"
  >
    <span class="size-2 shrink-0 rounded-full" :class="dot[group]" aria-hidden="true" />
    {{ level }}
  </span>
</template>
