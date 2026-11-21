<script setup lang="ts">
import type { HTMLAttributes } from 'vue'

import { cn } from '@/shared/lib/utils'

const props = defineProps<{ class?: HTMLAttributes['class'] }>()
</script>

<template>
  <td
    :class="
      cn(
        'h-11 border-b px-3 align-middle first:pl-4 last:pr-4 [tr:last-child>&]:border-b-0',
        props.class,
      )
    "
  >
    <slot />
  </td>
</template>
