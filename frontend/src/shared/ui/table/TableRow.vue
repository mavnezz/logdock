<script setup lang="ts">
import type { HTMLAttributes } from 'vue'

import { cn } from '@/shared/lib/utils'

const props = defineProps<{ class?: HTMLAttributes['class'] }>()
</script>

<template>
  <tr
    :class="
      cn('transition-colors hover:bg-accent/60 data-[state=selected]:bg-primary/5', props.class)
    "
  >
    <slot />
  </tr>
</template>
