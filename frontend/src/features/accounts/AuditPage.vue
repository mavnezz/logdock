<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { problemMessage } from '@/shared/i18n'
import { formatDateTime } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { PageHeader } from '@/shared/ui/page'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import { useAudit } from './api'

const { t } = useI18n()
const audit = useAudit()
const entries = computed(() => audit.data.value?.pages.flat() ?? [])
const variant = (outcome: string) =>
  outcome === 'success' ? 'success' : outcome === 'denied' ? 'destructive' : 'warning'
</script>

<template>
  <div class="grid gap-6">
    <PageHeader :title="t('audit.title')" :description="t('audit.description')" />
    <Alert v-if="audit.error.value" variant="destructive">{{
      problemMessage(audit.error.value)
    }}</Alert>
    <Table v-if="audit.data.value">
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('audit.at') }}</TableHead>
          <TableHead>{{ t('audit.action') }}</TableHead>
          <TableHead>{{ t('audit.actor') }}</TableHead>
          <TableHead>{{ t('audit.target') }}</TableHead>
          <TableHead>{{ t('audit.address') }}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow v-for="entry in entries" :key="entry.id">
          <TableCell class="whitespace-nowrap text-muted-foreground">{{
            formatDateTime(entry.at)
          }}</TableCell>
          <TableCell>
            <div class="flex items-center gap-2">
              <code class="font-mono text-xs">{{ entry.action }}</code>
              <Badge :variant="variant(entry.outcome)">{{
                t(`audit.outcomes.${entry.outcome}`)
              }}</Badge>
              <span v-if="entry.reason" class="text-xs text-muted-foreground">{{
                entry.reason
              }}</span>
            </div>
          </TableCell>
          <TableCell>{{ entry.actor ?? '–' }}</TableCell>
          <TableCell>{{ entry.target ?? '–' }}</TableCell>
          <TableCell class="font-mono text-xs text-muted-foreground">{{
            entry.client_ip ?? '–'
          }}</TableCell>
        </TableRow>
      </TableBody>
    </Table>
    <div v-if="audit.hasNextPage.value" class="flex justify-center">
      <Button
        variant="outline"
        size="sm"
        :disabled="audit.isFetchingNextPage.value"
        @click="audit.fetchNextPage()"
        >{{ t('common.older') }}</Button
      >
    </div>
  </div>
</template>
