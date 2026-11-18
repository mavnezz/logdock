<script setup lang="ts">
import { Plus } from 'lucide-vue-next'
import { reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useSessionStore } from '@/shared/session/store'
import type { Account, AccountRole } from '@/shared/api/types'
import { LANGUAGES, problemMessage } from '@/shared/i18n'
import { formatDateTime } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { ConfirmDialog } from '@/shared/ui/confirm-dialog'
import { ModalDialog } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { NativeSelect } from '@/shared/ui/native-select'
import { FormField, PageHeader } from '@/shared/ui/page'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import {
  useAccounts,
  useCreateAccount,
  useDeleteAccount,
  useResetPassword,
  useUpdateAccount,
} from './api'

const { t } = useI18n()
const store = useSessionStore()
const accounts = useAccounts()
const create = useCreateAccount()
const update = useUpdateAccount()
const reset = useResetPassword()
const remove = useDeleteAccount()

const creating = ref(false)
interface NewAccount {
  email: string
  name: string
  role: AccountRole
  password: string
  locale: string
}
const form = reactive<NewAccount>({
  email: '',
  name: '',
  role: 'member',
  password: '',
  locale: 'en',
})
const resetting = ref<Account | null>(null)
const newPassword = ref('')
const deleting = ref<Account | null>(null)

async function submit(): Promise<void> {
  await create.mutateAsync({ ...form })
  creating.value = false
  Object.assign(form, { email: '', name: '', role: 'member', password: '' })
}

async function submitReset(): Promise<void> {
  if (!resetting.value) return
  await reset.mutateAsync({ id: resetting.value.id, password: newPassword.value })
  resetting.value = null
  newPassword.value = ''
}

const isSelf = (account: Account) => account.id === store.account?.id
const error = () => update.error.value ?? remove.error.value
</script>

<template>
  <div class="grid gap-6">
    <PageHeader :title="t('accounts.title')" :description="t('accounts.description')">
      <Button @click="creating = true"><Plus />{{ t('accounts.create') }}</Button>
    </PageHeader>
    <Alert v-if="error()" variant="destructive">{{ problemMessage(error()) }}</Alert>

    <Table v-if="accounts.data.value">
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('auth.name') }}</TableHead>
          <TableHead>{{ t('auth.email') }}</TableHead>
          <TableHead>{{ t('accounts.role') }}</TableHead>
          <TableHead>{{ t('accounts.created') }}</TableHead>
          <TableHead />
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow v-for="account in accounts.data.value" :key="account.id">
          <TableCell class="font-medium">
            {{ account.name }}
            <Badge v-if="isSelf(account)" variant="outline" class="ml-1">{{
              t('accounts.you')
            }}</Badge>
            <Badge v-if="account.disabled" variant="warning" class="ml-1">{{
              t('accounts.disabled')
            }}</Badge>
          </TableCell>
          <TableCell>{{ account.email }}</TableCell>
          <TableCell>
            <NativeSelect
              v-if="!isSelf(account)"
              :model-value="account.role"
              class="w-32"
              @update:model-value="
                (role) => update.mutate({ id: account.id, role: role as AccountRole })
              "
            >
              <option value="member">{{ t('accounts.roles.member') }}</option>
              <option value="admin">{{ t('accounts.roles.admin') }}</option>
            </NativeSelect>
            <span v-else>{{ t(`accounts.roles.${account.role}`) }}</span>
          </TableCell>
          <TableCell class="text-muted-foreground">{{
            formatDateTime(account.created_at)
          }}</TableCell>
          <TableCell class="text-right whitespace-nowrap">
            <template v-if="!isSelf(account)">
              <Button
                size="sm"
                variant="ghost"
                @click="update.mutate({ id: account.id, disabled: !account.disabled })"
              >
                {{ account.disabled ? t('accounts.enable') : t('accounts.disable') }}
              </Button>
              <Button size="sm" variant="ghost" @click="resetting = account">{{
                t('accounts.resetPassword')
              }}</Button>
              <Button size="sm" variant="ghost" @click="deleting = account">{{
                t('common.delete')
              }}</Button>
            </template>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>

    <ModalDialog
      v-model:open="creating"
      :title="t('accounts.create')"
      :close-label="t('common.close')"
    >
      <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
        <FormField :label="t('auth.name')" for="account-name"
          ><Input id="account-name" v-model="form.name" required
        /></FormField>
        <FormField :label="t('auth.email')" for="account-email">
          <Input id="account-email" v-model="form.email" type="email" required />
        </FormField>
        <FormField :label="t('accounts.role')" for="account-role" :hint="t('accounts.roleHint')">
          <NativeSelect id="account-role" v-model="form.role">
            <option value="member">{{ t('accounts.roles.member') }}</option>
            <option value="admin">{{ t('accounts.roles.admin') }}</option>
          </NativeSelect>
        </FormField>
        <FormField :label="t('profile.language')" for="account-locale">
          <NativeSelect id="account-locale" v-model="form.locale">
            <option v-for="language in LANGUAGES" :key="language.locale" :value="language.locale">
              {{ language.name }}
            </option>
          </NativeSelect>
        </FormField>
        <FormField
          :label="t('auth.password')"
          for="account-password"
          :hint="t('auth.passwordRule')"
        >
          <Input
            id="account-password"
            v-model="form.password"
            type="password"
            autocomplete="new-password"
            required
            minlength="10"
          />
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
      :open="resetting !== null"
      :title="t('accounts.resetPassword')"
      :description="resetting?.email"
      :close-label="t('common.close')"
      @update:open="(open) => !open && (resetting = null)"
    >
      <form class="grid gap-4" @submit.prevent="submitReset().catch(() => undefined)">
        <FormField
          :label="t('profile.newPassword')"
          for="reset-password"
          :hint="t('accounts.resetHint')"
        >
          <Input
            id="reset-password"
            v-model="newPassword"
            type="password"
            autocomplete="new-password"
            required
            minlength="10"
          />
        </FormField>
        <Alert v-if="reset.error.value" variant="destructive">{{
          problemMessage(reset.error.value)
        }}</Alert>
        <div class="flex justify-end">
          <Button type="submit" :disabled="reset.isPending.value">{{ t('common.save') }}</Button>
        </div>
      </form>
    </ModalDialog>

    <ConfirmDialog
      :open="deleting !== null"
      :title="t('accounts.deleteTitle', { name: deleting?.name ?? '' })"
      :description="t('accounts.deleteDescription')"
      :confirm-label="t('common.delete')"
      :cancel-label="t('common.cancel')"
      destructive
      @update:open="(open) => !open && (deleting = null)"
      @confirm="deleting && remove.mutate(deleting.id)"
    />
  </div>
</template>
