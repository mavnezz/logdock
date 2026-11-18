<script setup lang="ts">
defineProps<{ title: string; description?: string }>()
</script>

<!-- What a list shows while it has nothing, and how to change that. -->
<template>
  <div class="grid place-items-center gap-2 rounded-xl border border-dashed p-10 text-center">
    <p class="font-medium">{{ title }}</p>
    <p v-if="description" class="max-w-md text-sm text-muted-foreground">{{ description }}</p>
    <div v-if="$slots.default" class="mt-2"><slot /></div>
  </div>
</template>
