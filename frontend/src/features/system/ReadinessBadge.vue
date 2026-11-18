<script setup lang="ts">
import { CircleAlert, CircleCheck } from 'lucide-vue-next'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useReadiness } from './api'

const i18n = useI18n()
const { t } = i18n
const readiness = useReadiness()

const problems = computed(() =>
  (readiness.data.value?.problems ?? []).map((code) =>
    i18n.te(`system.problem.${code}`) ? t(`system.problem.${code}`) : code,
  ),
)
const ready = computed(() => readiness.data.value?.status === 'ready')
</script>

<!-- Ready or not, with the reasons when not. -->
<template>
  <div class="flex items-start gap-2 text-sm" role="status">
    <template v-if="readiness.isPending.value">
      <span class="text-muted-foreground">{{ t('common.loading') }}</span>
    </template>
    <template v-else-if="ready">
      <CircleCheck class="size-4 shrink-0 text-success" />
      <span>{{ t('system.ready') }}</span>
    </template>
    <template v-else>
      <CircleAlert class="size-4 shrink-0 text-destructive" />
      <div>
        <p class="font-medium">{{ t('system.notReady') }}</p>
        <ul class="text-muted-foreground">
          <li v-for="problem in problems" :key="problem">{{ problem }}</li>
        </ul>
      </div>
    </template>
  </div>
</template>
