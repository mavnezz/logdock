<script setup lang="ts">
import { Copy, KeyRound, Plus } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { problemMessage } from '@/shared/i18n'
import type { CreatedKey, Project } from '@/shared/api/types'
import { formatDateTime, formatRelative } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ConfirmDialog } from '@/shared/ui/confirm-dialog'
import { ModalDialog } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { NativeSelect } from '@/shared/ui/native-select'
import { EmptyState, FormField } from '@/shared/ui/page'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import { useCreateKey, useKeys, useRevokeKey } from './api'
import { laravelEnv } from './laravel'

const props = defineProps<{ project: Project }>()
const { t } = useI18n()
const keys = useKeys(() => props.project.id)
const create = useCreateKey(() => props.project.id)
const revoke = useRevokeKey(() => props.project.id)

const creating = ref(false)
// '': a key of the whole project, whose data names its stage.
const environment = ref<number | ''>('')
const label = ref('')
const created = ref<CreatedKey | null>(null)
const revoking = ref<number | null>(null)
const copied = ref(false)

const environmentName = (id: number | null | undefined) =>
  id == null ? null : (props.project.environments.find((e) => e.id === id)?.name ?? '–')
const snippet = computed(() =>
  created.value
    ? laravelEnv(created.value.dsn, environmentName(created.value.key.environment_id))
    : '',
)

async function submit(): Promise<void> {
  created.value = await create.mutateAsync({
    environment_id: environment.value === '' ? null : environment.value,
    label: label.value,
  })
  creating.value = false
  label.value = ''
}

async function copy(text: string): Promise<void> {
  await navigator.clipboard.writeText(text)
  copied.value = true
  setTimeout(() => (copied.value = false), 1500)
}
</script>

<template>
  <section class="grid gap-3">
    <div class="flex items-center justify-between gap-2">
      <h2 class="font-medium">{{ t('keys.title') }}</h2>
      <Button size="sm" @click="creating = true"> <Plus />{{ t('keys.create') }} </Button>
    </div>
    <p class="text-sm text-muted-foreground">{{ t('keys.description') }}</p>

    <EmptyState v-if="keys.data.value?.length === 0" :title="t('keys.empty')" />
    <Table v-else-if="keys.data.value">
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('keys.key') }}</TableHead>
          <TableHead>{{ t('keys.environment') }}</TableHead>
          <TableHead>{{ t('keys.lastUsed') }}</TableHead>
          <TableHead>{{ t('keys.created') }}</TableHead>
          <TableHead />
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow
          v-for="key in keys.data.value"
          :key="key.id"
          :class="key.revoked_at && 'opacity-60'"
        >
          <TableCell>
            <div class="flex items-center gap-2">
              <KeyRound class="size-4 text-muted-foreground" />
              <code class="font-mono text-xs">{{ key.prefix }}…</code>
              <span v-if="key.label" class="text-muted-foreground">{{ key.label }}</span>
            </div>
          </TableCell>
          <TableCell>
            <Badge :variant="key.environment_id == null ? 'outline' : 'secondary'">{{
              environmentName(key.environment_id) ?? t('keys.allStages')
            }}</Badge>
          </TableCell>
          <TableCell class="text-muted-foreground">
            {{ key.last_used_at ? formatRelative(key.last_used_at) : t('keys.never') }}
          </TableCell>
          <TableCell class="text-muted-foreground">{{ formatDateTime(key.created_at) }}</TableCell>
          <TableCell class="text-right">
            <Badge v-if="key.revoked_at" variant="outline">{{ t('keys.revoked') }}</Badge>
            <Button v-else size="sm" variant="ghost" @click="revoking = key.id">{{
              t('keys.revoke')
            }}</Button>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>

    <ModalDialog v-model:open="creating" :title="t('keys.create')" :close-label="t('common.close')">
      <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
        <FormField :label="t('keys.environment')" for="key-env" :hint="t('keys.environmentHint')">
          <NativeSelect id="key-env" v-model="environment">
            <option value="">{{ t('keys.allStages') }}</option>
            <option v-for="env in project.environments" :key="env.id" :value="env.id">
              {{ env.name }}
            </option>
          </NativeSelect>
        </FormField>
        <FormField :label="t('keys.label')" for="key-label" :hint="t('keys.labelHint')">
          <Input id="key-label" v-model="label" />
        </FormField>
        <Alert v-if="create.error.value" variant="destructive">{{
          problemMessage(create.error.value)
        }}</Alert>
        <div class="flex justify-end gap-2">
          <Button type="button" variant="outline" @click="creating = false">{{
            t('common.cancel')
          }}</Button>
          <Button type="submit" :disabled="create.isPending.value">{{ t('common.create') }}</Button>
        </div>
      </form>
    </ModalDialog>

    <ModalDialog
      :open="created !== null"
      :title="t('keys.createdTitle')"
      :description="t('keys.createdDescription')"
      :close-label="t('common.close')"
      wide
      @update:open="(open) => !open && (created = null)"
    >
      <div v-if="created" class="grid gap-4">
        <FormField label="DSN" for="dsn">
          <div class="flex gap-2">
            <Input id="dsn" :model-value="created.dsn" readonly class="font-mono text-xs" />
            <Button
              variant="outline"
              size="icon"
              :aria-label="t('keys.copy')"
              @click="copy(created.dsn)"
              ><Copy
            /></Button>
          </div>
        </FormField>
        <div class="grid gap-1.5">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium">{{ t('keys.laravel') }}</span>
            <Button variant="ghost" size="sm" @click="copy(snippet)"
              ><Copy />{{ copied ? t('keys.copied') : t('keys.copy') }}</Button
            >
          </div>
          <pre class="overflow-x-auto rounded-md bg-muted p-3 font-mono text-xs">{{ snippet }}</pre>
          <p class="text-xs text-muted-foreground">{{ t('keys.laravelHint') }}</p>
        </div>
      </div>
    </ModalDialog>

    <ConfirmDialog
      :open="revoking !== null"
      :title="t('keys.revokeTitle')"
      :description="t('keys.revokeDescription')"
      :confirm-label="t('keys.revoke')"
      :cancel-label="t('common.cancel')"
      destructive
      @update:open="(open) => !open && (revoking = null)"
      @confirm="revoking !== null && revoke.mutate(revoking)"
    />
  </section>
</template>
