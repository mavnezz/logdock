<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import Skeleton from './Skeleton.vue'

/** `rows` lines of a list, or `tiles` figures above it. */
withDefaults(defineProps<{ rows?: number; tiles?: number }>(), { rows: 6, tiles: 0 })
const { t } = useI18n()
</script>

<!-- A page's content while it loads: its shape, and the word for screen readers. -->
<template>
  <div class="grid gap-4" role="status">
    <span class="sr-only">{{ t('common.loading') }}</span>
    <div v-if="tiles" class="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Skeleton v-for="n in tiles" :key="n" class="h-24 rounded-xl" />
    </div>
    <div class="grid gap-px overflow-hidden rounded-xl border">
      <div v-for="n in rows" :key="n" class="flex items-center gap-3 bg-card px-4 py-3">
        <Skeleton class="size-2 rounded-full" />
        <Skeleton class="h-3.5" :style="{ width: `${40 + ((n * 37) % 45)}%` }" />
        <Skeleton class="ml-auto h-3.5 w-16" />
      </div>
    </div>
  </div>
</template>
