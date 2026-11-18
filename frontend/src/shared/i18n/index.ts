import { createI18n } from 'vue-i18n'

import { ApiProblem } from '@/shared/api/client'

import de from './de'
import en, { type Messages } from './en'

export type Locale = 'de' | 'en'

const STORAGE_KEY = 'logdock.locale'

/** English first; German once chosen, or when the browser asks for it. */
export function detectLocale(
  stored: string | null,
  browser: readonly string[] = navigator.languages,
): Locale {
  if (stored === 'de' || stored === 'en') return stored
  return browser[0]?.toLowerCase().startsWith('de') ? 'de' : 'en'
}

function storedLocale(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

const locale = detectLocale(storedLocale())
document.documentElement.lang = locale

export const i18n = createI18n<[Messages], Locale, false>({
  legacy: false,
  locale,
  fallbackLocale: 'en',
  messages: { de, en },
})

/** Languages offered in the settings, each named in itself. */
export const LANGUAGES: { locale: Locale; name: string }[] = [
  { locale: 'en', name: 'English' },
  { locale: 'de', name: 'Deutsch' },
]

export function setLocale(next: Locale): void {
  i18n.global.locale.value = next
  document.documentElement.lang = next
  try {
    localStorage.setItem(STORAGE_KEY, next)
  } catch {
    // Storage unavailable: applies to this page only.
  }
}

/** A problem as a sentence of the current language. */
export function problemMessage(error: unknown): string {
  const t = i18n.global.t
  if (error instanceof ApiProblem) {
    const key = `problems.${error.code}`
    if (i18n.global.te(key)) return t(key)
    if (error.detail) return error.detail
  }
  return t('problems.unknown')
}
