import { afterEach, describe, expect, it, vi } from 'vitest'

import { applyTheme, followSystemTheme, setThemePreference, themePreference } from './theme'

function systemPrefersDark(dark: boolean) {
  const listeners: (() => void)[] = []
  vi.stubGlobal('matchMedia', () => ({
    matches: dark,
    addEventListener: (_: string, listener: () => void) => listeners.push(listener),
  }))
  return listeners
}

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

describe('the colour scheme', () => {
  it('follows the system until one is chosen', () => {
    const listeners = systemPrefersDark(true)
    expect(themePreference()).toBe('system')
    applyTheme()
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    followSystemTheme()
    expect(listeners).toHaveLength(1)

    setThemePreference('light')
    expect(themePreference()).toBe('light')
    expect(document.documentElement.classList.contains('dark')).toBe(false)

    setThemePreference('system')
    expect(themePreference()).toBe('system')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })
})
