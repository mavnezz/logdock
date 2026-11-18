<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'

import { problemMessage } from '@/shared/i18n'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { FormField } from '@/shared/ui/page'

import { useLogin } from './api'
import AuthFrame from './AuthFrame.vue'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const login = useLogin()
const email = ref('')
const password = ref('')

async function submit(): Promise<void> {
  await login.mutateAsync({ email: email.value, password: password.value })
  const next =
    typeof route.query.next === 'string' && route.query.next.startsWith('/')
      ? route.query.next
      : '/'
  await router.replace(next)
}
</script>

<template>
  <AuthFrame :title="t('auth.signIn')">
    <form class="grid gap-4" @submit.prevent="submit().catch(() => undefined)">
      <FormField :label="t('auth.email')" for="email">
        <Input id="email" v-model="email" type="email" autocomplete="username" required autofocus />
      </FormField>
      <FormField :label="t('auth.password')" for="password">
        <Input
          id="password"
          v-model="password"
          type="password"
          autocomplete="current-password"
          required
        />
      </FormField>
      <Alert v-if="login.error.value" variant="destructive">{{
        problemMessage(login.error.value)
      }}</Alert>
      <Button type="submit" :disabled="login.isPending.value">{{ t('auth.signIn') }}</Button>
    </form>
  </AuthFrame>
</template>
