<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { LANGUAGES, problemMessage } from '@/shared/i18n'
import { setThemePreference, type ThemePreference, themePreference } from '@/shared/lib/theme'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { NativeSelect } from '@/shared/ui/native-select'
import { FormField, PageHeader } from '@/shared/ui/page'

import { useChangePassword, useUpdateProfile } from './api'
import { useSessionStore } from '@/shared/session/store'

const { t } = useI18n()
const store = useSessionStore()
const profile = useUpdateProfile()
const password = useChangePassword()

const form = reactive({ name: store.account?.name ?? '', locale: store.account?.locale ?? 'en' })
const theme = ref<ThemePreference>(themePreference())
watch(theme, (next) => setThemePreference(next))
const passwords = reactive({ current: '', next: '' })
const passwordChanged = ref(false)

async function saveProfile(): Promise<void> {
  await profile.mutateAsync({ name: form.name, locale: form.locale })
}

async function savePassword(): Promise<void> {
  passwordChanged.value = false
  await password.mutateAsync({ current: passwords.current, new: passwords.next })
  passwords.current = ''
  passwords.next = ''
  passwordChanged.value = true
}
</script>

<template>
  <div class="grid max-w-xl gap-8">
    <PageHeader :title="t('profile.title')" :description="store.account?.email" />

    <form class="grid gap-4" @submit.prevent="saveProfile().catch(() => undefined)">
      <h2 class="font-medium">{{ t('profile.details') }}</h2>
      <FormField :label="t('auth.name')" for="profile-name">
        <Input id="profile-name" v-model="form.name" required />
      </FormField>
      <FormField :label="t('profile.language')" for="profile-locale">
        <NativeSelect id="profile-locale" v-model="form.locale">
          <option v-for="language in LANGUAGES" :key="language.locale" :value="language.locale">
            {{ language.name }}
          </option>
        </NativeSelect>
      </FormField>
      <FormField :label="t('profile.theme')" for="profile-theme" :hint="t('profile.themeHint')">
        <NativeSelect id="profile-theme" v-model="theme">
          <option value="system">{{ t('profile.themes.system') }}</option>
          <option value="light">{{ t('profile.themes.light') }}</option>
          <option value="dark">{{ t('profile.themes.dark') }}</option>
        </NativeSelect>
      </FormField>
      <Alert v-if="profile.error.value" variant="destructive">{{
        problemMessage(profile.error.value)
      }}</Alert>
      <div>
        <Button type="submit" :disabled="profile.isPending.value">{{ t('common.save') }}</Button>
      </div>
    </form>

    <form class="grid gap-4" @submit.prevent="savePassword().catch(() => undefined)">
      <h2 class="font-medium">{{ t('profile.password') }}</h2>
      <FormField :label="t('profile.currentPassword')" for="current-password">
        <Input
          id="current-password"
          v-model="passwords.current"
          type="password"
          autocomplete="current-password"
          required
        />
      </FormField>
      <FormField
        :label="t('profile.newPassword')"
        for="new-password"
        :hint="t('auth.passwordRule')"
      >
        <Input
          id="new-password"
          v-model="passwords.next"
          type="password"
          autocomplete="new-password"
          required
          minlength="10"
        />
      </FormField>
      <Alert v-if="password.error.value" variant="destructive">{{
        problemMessage(password.error.value)
      }}</Alert>
      <Alert v-if="passwordChanged">{{ t('profile.passwordChanged') }}</Alert>
      <div>
        <Button type="submit" :disabled="password.isPending.value">{{
          t('profile.changePassword')
        }}</Button>
      </div>
    </form>
  </div>
</template>
