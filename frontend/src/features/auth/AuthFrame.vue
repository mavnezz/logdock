<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ title: string; description?: string }>()
const { t } = useI18n()
</script>

<!-- The centred card of the sign-in and setup pages. -->
<template>
  <main class="grid min-h-dvh place-items-center bg-muted/40 p-4">
    <div class="grid w-full max-w-sm gap-6">
      <div class="flex items-center justify-center gap-2">
        <img src="/favicon.svg" alt="" class="size-8" />
        <span class="text-lg font-semibold">{{ t('app.name') }}</span>
      </div>
      <section class="grid gap-5 rounded-xl border bg-card p-6 shadow-sm">
        <div class="grid gap-1">
          <h1 class="text-lg font-semibold">{{ title }}</h1>
          <p v-if="description" class="text-sm text-muted-foreground">{{ description }}</p>
        </div>
        <slot />
      </section>
    </div>
  </main>
</template>
