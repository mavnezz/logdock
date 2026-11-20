<script setup lang="ts">
import { ChevronLeft, ChevronRight } from 'lucide-vue-next'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'

/** Pages through a list: the page (from 1), and whether another follows. */
defineProps<{ page: number; hasNext: boolean }>()
const emit = defineEmits<{ 'update:page': [page: number] }>()
const { t } = useI18n()
</script>

<template>
  <nav
    v-if="page > 1 || hasNext"
    class="flex items-center justify-center gap-2"
    :aria-label="t('common.pages')"
  >
    <Button variant="outline" size="sm" :disabled="page <= 1" @click="emit('update:page', page - 1)"
      ><ChevronLeft />{{ t('common.previous') }}</Button
    >
    <span class="px-2 text-sm text-muted-foreground tabular-nums">{{
      t('common.page', { page })
    }}</span>
    <Button variant="outline" size="sm" :disabled="!hasNext" @click="emit('update:page', page + 1)"
      >{{ t('common.next') }}<ChevronRight
    /></Button>
  </nav>
</template>
