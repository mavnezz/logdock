<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import ReadinessBadge from '@/features/system/ReadinessBadge.vue'

const { t } = useI18n()
</script>

<template>
  <main class="grid min-h-dvh place-items-center p-6">
    <div class="grid max-w-sm gap-4 text-center">
      <img src="/favicon.svg" alt="" class="mx-auto size-14" />
      <h1 class="text-2xl font-semibold">{{ t('app.name') }}</h1>
      <p class="text-muted-foreground">{{ t('app.tagline') }}</p>
      <div class="mx-auto"><ReadinessBadge /></div>
    </div>
  </main>
</template>
