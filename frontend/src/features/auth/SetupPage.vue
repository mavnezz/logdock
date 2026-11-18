<script setup lang="ts">
import { reactive } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { i18n, LANGUAGES, problemMessage } from '@/shared/i18n'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { NativeSelect } from '@/shared/ui/native-select'
import { FormField } from '@/shared/ui/page'

import { useSetup } from './api'
import AuthFrame from './AuthFrame.vue'

const { t } = useI18n()
const router = useRouter()
const setup = useSetup()
const form = reactive({
  code: '',
  name: '',
  email: '',
  password: '',
  locale: i18n.global.locale.value,
})

async function submit(): Promise<void> {
  await setup.mutateAsync({ ...form })
  await router.replace('/projects')
}
</script>

<template>
  <AuthFrame :title="t('setup.title')" :description="t('setup.description')">
    <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
      <FormField :label="t('setup.code')" for="code" :hint="t('setup.codeHint')">
        <Input
          id="code"
          v-model="form.code"
          autocomplete="one-time-code"
          required
          autofocus
          class="font-mono uppercase"
        />
      </FormField>
      <FormField :label="t('auth.name')" for="name">
        <Input id="name" v-model="form.name" autocomplete="name" required />
      </FormField>
      <FormField :label="t('auth.email')" for="email">
        <Input id="email" v-model="form.email" type="email" autocomplete="username" required />
      </FormField>
      <FormField :label="t('auth.password')" for="password" :hint="t('auth.passwordRule')">
        <Input
          id="password"
          v-model="form.password"
          type="password"
          autocomplete="new-password"
          required
          minlength="10"
        />
      </FormField>
      <FormField :label="t('profile.language')" for="locale">
        <NativeSelect id="locale" v-model="form.locale">
          <option v-for="language in LANGUAGES" :key="language.locale" :value="language.locale">
            {{ language.name }}
          </option>
        </NativeSelect>
      </FormField>
      <Alert v-if="setup.error.value" variant="destructive">{{
        problemMessage(setup.error.value)
      }}</Alert>
      <Button type="submit" :disabled="setup.isPending.value">{{ t('setup.submit') }}</Button>
    </form>
  </AuthFrame>
</template>
