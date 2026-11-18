<script setup lang="ts">
defineProps<{ title: string; description?: string }>()
</script>

<!-- A page's title, what it is for, and its main actions on the right. -->
<template>
  <header class="flex flex-wrap items-start justify-between gap-3">
    <div class="grid gap-1">
      <h1 class="text-xl font-semibold tracking-tight">{{ title }}</h1>
      <p v-if="description" class="max-w-3xl text-sm text-muted-foreground">{{ description }}</p>
    </div>
    <div v-if="$slots.default" class="flex flex-wrap items-center gap-2">
      <slot />
    </div>
  </header>
</template>
