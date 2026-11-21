<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useReadiness } from './api'

/** `compact`: the light alone, its words as its title. */
defineProps<{ compact?: boolean }>()
const i18n = useI18n()
const { t } = i18n
const readiness = useReadiness()

const problems = computed(() =>
  (readiness.data.value?.problems ?? []).map((code) =>
    i18n.te(`system.problem.${code}`) ? t(`system.problem.${code}`) : code,
  ),
)
const ready = computed(() => readiness.data.value?.status === 'ready')
const label = computed(() =>
  readiness.isPending.value
    ? t('common.loading')
    : ready.value
      ? t('system.ready')
      : t('system.notReady'),
)
</script>

<!-- Ready or not, with the reasons when not. -->
<template>
  <div
    class="flex items-start gap-2 text-xs"
    :class="compact && 'justify-center'"
    role="status"
    :title="compact ? [label, ...problems].join('\n') : undefined"
  >
    <span class="relative mt-1 flex size-2 shrink-0" aria-hidden="true">
      <span
        v-if="ready"
        class="absolute inset-0 animate-ping rounded-full bg-success opacity-40 [animation-duration:2.5s]"
      />
      <span
        class="relative size-2 rounded-full"
        :class="
          readiness.isPending.value
            ? 'bg-muted-foreground/40'
            : ready
              ? 'bg-success'
              : 'bg-destructive'
        "
      />
    </span>
    <div v-if="!compact" class="grid min-w-0 gap-0.5">
      <span :class="ready || readiness.isPending.value ? 'text-muted-foreground' : 'font-medium'">{{
        label
      }}</span>
      <ul v-if="!ready && problems.length" class="text-muted-foreground">
        <li v-for="problem in problems" :key="problem">{{ problem }}</li>
      </ul>
    </div>
    <span v-else class="sr-only">{{ label }}</span>
  </div>
</template>
