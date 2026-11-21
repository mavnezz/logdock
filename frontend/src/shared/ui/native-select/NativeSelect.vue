<script setup lang="ts">
import { ChevronDown } from 'lucide-vue-next'
import type { HTMLAttributes } from 'vue'

import { cn } from '@/shared/lib/utils'

// aria-label and the like belong to the select, not its frame.
defineOptions({ inheritAttrs: false })
const props = defineProps<{ id?: string; disabled?: boolean; class?: HTMLAttributes['class'] }>()
const value = defineModel<string | number>({ required: true })
</script>

<!--
  The browser's own list: fast with hundreds of entries (time zones), typed
  into to jump, and native on phones.
-->
<template>
  <div :class="cn('relative', props.class)">
    <select
      v-bind="$attrs"
      :id="id"
      v-model="value"
      :disabled="disabled"
      data-slot="native-select"
      class="h-8 w-full cursor-pointer appearance-none truncate rounded-lg border border-input bg-card py-1 pr-8 pl-3 text-sm shadow-xs transition-[border-color,box-shadow] outline-none hover:border-foreground/20 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-input/20"
    >
      <slot />
    </select>
    <ChevronDown
      class="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
    />
  </div>
</template>
