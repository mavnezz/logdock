<script setup lang="ts">
import { Inbox, type LucideIcon } from 'lucide-vue-next'

withDefaults(defineProps<{ title: string; description?: string; icon?: LucideIcon }>(), {
  description: undefined,
  icon: () => Inbox,
})
</script>

<!-- What a list shows while it has nothing, and how to change that. -->
<template>
  <div
    class="grid place-items-center gap-3 rounded-xl border border-dashed bg-card/50 px-6 py-12 text-center"
  >
    <span
      class="grid size-10 place-items-center rounded-xl border bg-card text-muted-foreground shadow-xs"
      aria-hidden="true"
    >
      <component :is="icon" class="size-5" />
    </span>
    <div class="grid gap-1">
      <p class="font-medium">{{ title }}</p>
      <p v-if="description" class="max-w-md text-sm text-pretty text-muted-foreground">
        {{ description }}
      </p>
    </div>
    <div v-if="$slots.default" class="mt-1"><slot /></div>
  </div>
</template>
