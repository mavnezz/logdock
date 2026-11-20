<script setup lang="ts">
import { Copy, Plus, Terminal } from 'lucide-vue-next'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { CreatedToken } from '@/shared/api/types'
import { problemMessage } from '@/shared/i18n'
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

import { useCreateToken, useRevokeToken, useTokens } from './api'

const { t } = useI18n()
const tokens = useTokens()
const create = useCreateToken()
const revoke = useRevokeToken()

const creating = ref(false)
const name = ref('')
const scope = ref<'read' | 'triage'>('read')
const created = ref<CreatedToken | null>(null)
const revoking = ref<number | null>(null)
const copied = ref<string | null>(null)

/** How Claude Code is told about Logdock's MCP server. */
const claudeCommand = computed(() =>
  created.value
    ? `claude mcp add --transport http logdock ${created.value.mcp_url} --header "Authorization: Bearer ${created.value.secret}"`
    : '',
)
const curlCommand = computed(() =>
  created.value
    ? `curl -H "Authorization: Bearer ${created.value.secret}" ${created.value.mcp_url.replace(/\/mcp$/, '')}/api/v1/issues`
    : '',
)

async function submit(): Promise<void> {
  created.value = await create.mutateAsync({ name: name.value, scope: scope.value })
  creating.value = false
  name.value = ''
  scope.value = 'read'
}

async function copy(what: string, text: string): Promise<void> {
  await navigator.clipboard.writeText(text)
  copied.value = what
  setTimeout(() => (copied.value = null), 1500)
}
</script>

<template>
  <section class="grid gap-3">
    <div class="flex items-center justify-between gap-2">
      <h2 class="font-medium">{{ t('tokens.title') }}</h2>
      <Button size="sm" @click="creating = true"><Plus />{{ t('tokens.create') }}</Button>
    </div>
    <p class="text-sm text-muted-foreground">{{ t('tokens.description') }}</p>

    <EmptyState v-if="tokens.data.value?.length === 0" :title="t('tokens.empty')" />
    <Table v-else-if="tokens.data.value">
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('tokens.name') }}</TableHead>
          <TableHead>{{ t('tokens.scope') }}</TableHead>
          <TableHead>{{ t('tokens.lastUsed') }}</TableHead>
          <TableHead />
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow
          v-for="token in tokens.data.value"
          :key="token.id"
          :class="token.revoked_at && 'opacity-60'"
        >
          <TableCell>
            <div class="grid">
              <span>{{ token.name }}</span>
              <code class="font-mono text-xs text-muted-foreground">{{ token.prefix }}…</code>
            </div>
          </TableCell>
          <TableCell>
            <Badge :variant="token.scope === 'triage' ? 'default' : 'secondary'">{{
              t(`tokens.scopes.${token.scope}`)
            }}</Badge>
          </TableCell>
          <TableCell class="text-muted-foreground" :title="formatDateTime(token.created_at)">
            {{ token.last_used_at ? formatRelative(token.last_used_at) : t('tokens.never') }}
          </TableCell>
          <TableCell class="text-right">
            <Badge v-if="token.revoked_at" variant="outline">{{ t('tokens.revoked') }}</Badge>
            <Button v-else size="sm" variant="ghost" @click="revoking = token.id">{{
              t('tokens.revoke')
            }}</Button>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>

    <ModalDialog
      v-model:open="creating"
      :title="t('tokens.create')"
      :close-label="t('common.close')"
    >
      <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
        <FormField :label="t('tokens.name')" for="token-name" :hint="t('tokens.nameHint')">
          <Input id="token-name" v-model="name" required maxlength="64" />
        </FormField>
        <FormField :label="t('tokens.scope')" for="token-scope" :hint="t('tokens.scopeHint')">
          <NativeSelect id="token-scope" v-model="scope">
            <option value="read">{{ t('tokens.scopes.read') }}</option>
            <option value="triage">{{ t('tokens.scopes.triage') }}</option>
          </NativeSelect>
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
      :title="t('tokens.createdTitle')"
      :description="t('tokens.createdDescription')"
      :close-label="t('common.close')"
      wide
      @update:open="(open) => !open && (created = null)"
    >
      <div v-if="created" class="grid gap-4">
        <FormField :label="t('tokens.token')" for="token-secret">
          <div class="flex gap-2">
            <Input
              id="token-secret"
              :model-value="created.secret"
              readonly
              class="font-mono text-xs"
            />
            <Button
              variant="outline"
              size="icon"
              :aria-label="t('tokens.copy')"
              @click="copy('secret', created.secret)"
              ><Copy
            /></Button>
          </div>
        </FormField>
        <div class="grid gap-1.5">
          <div class="flex items-center justify-between">
            <span class="flex items-center gap-1.5 text-sm font-medium"
              ><Terminal class="size-4" />{{ t('tokens.claude') }}</span
            >
            <Button variant="ghost" size="sm" @click="copy('claude', claudeCommand)"
              ><Copy />{{ copied === 'claude' ? t('tokens.copied') : t('tokens.copy') }}</Button
            >
          </div>
          <pre
            class="overflow-x-auto rounded-md bg-muted p-3 font-mono text-xs whitespace-pre-wrap break-all"
            >{{ claudeCommand }}</pre>
          <p class="text-xs text-muted-foreground">{{ t('tokens.claudeHint') }}</p>
        </div>
        <div class="grid gap-1.5">
          <span class="text-sm font-medium">{{ t('tokens.api') }}</span>
          <pre
            class="overflow-x-auto rounded-md bg-muted p-3 font-mono text-xs whitespace-pre-wrap break-all"
            >{{ curlCommand }}</pre>
          <p class="text-xs text-muted-foreground">{{ t('tokens.apiHint') }}</p>
        </div>
      </div>
    </ModalDialog>

    <ConfirmDialog
      :open="revoking !== null"
      :title="t('tokens.revokeTitle')"
      :description="t('tokens.revokeDescription')"
      :confirm-label="t('tokens.revoke')"
      :cancel-label="t('common.cancel')"
      destructive
      @update:open="(open) => !open && (revoking = null)"
      @confirm="revoking !== null && revoke.mutate(revoking)"
    />
  </section>
</template>
