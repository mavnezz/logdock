import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { themeChoice } from '@/shared/lib/theme'
import { useScopeStore } from '@/shared/scope/store'
import { mountWith, stubApi } from '@/shared/testing'
import { project } from '@/shared/testing/fixtures'

import CommandPalette from './CommandPalette.vue'
import { navigation } from './navigation'

let mounted: VueWrapper | null = null

afterEach(() => {
  mounted?.unmount()
  mounted = null
  vi.unstubAllGlobals()
  document.body.innerHTML = ''
  localStorage.clear()
})

function input(): HTMLInputElement {
  return document.body.querySelector<HTMLInputElement>('input[role="combobox"]')!
}

function options(): string[] {
  return [...document.body.querySelectorAll('[role="option"] [data-label]')].map((o) =>
    o.textContent.trim(),
  )
}

/** Types into the menu, opening it again after a command closed it. */
async function type(text: string): Promise<void> {
  if (!input()) {
    await mounted!.setProps({ open: true })
    await flushPromises()
  }
  input().value = text
  input().dispatchEvent(new Event('input'))
  await flushPromises()
}

async function press(key: string): Promise<void> {
  input().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
  await flushPromises()
}

async function open() {
  stubApi({ 'GET /api/v1/projects': () => [project()] })
  const signOut = vi.fn(() => Promise.resolve())
  const result = mountWith(CommandPalette, {
    props: {
      open: true,
      items: navigation,
      signOut,
      'onUpdate:open': (open: boolean) => void mounted?.setProps({ open }),
    },
    attachTo: document.body,
  })
  mounted = result.wrapper
  await flushPromises()
  return { ...result, signOut }
}

describe('the command menu', () => {
  it('goes to a page by its name', async () => {
    const { router } = await open()
    expect(options()).toContain('Logs')
    await type('sched')
    expect(options()[0]).toBe('Scheduled tasks')
    await press('Enter')
    expect(router.currentRoute.value.path).toBe('/monitors')
  })

  it('opens an issue by its number and a trace by its ID', async () => {
    const { router } = await open()
    await type('#42')
    expect(options()[0]).toBe('Issue #42')
    await press('Enter')
    expect(router.currentRoute.value.path).toBe('/issues/42')

    await type('0123456789abcdef0123456789ABCDEF')
    await press('Enter')
    expect(router.currentRoute.value.path).toBe('/traces/0123456789abcdef0123456789abcdef')
  })

  it('searches the logs or the issues for what was typed', async () => {
    const { router } = await open()
    await type('payment failed')
    expect(options()).toEqual([
      'Search the logs for “payment failed”',
      'Search the issues for “payment failed”',
    ])
    await press('ArrowDown')
    await press('ArrowDown')
    await press('ArrowUp')
    expect(document.body.querySelector('[aria-selected="true"]')!.textContent).toContain('issues')
    await press('Enter')
    expect(router.currentRoute.value.fullPath).toBe('/issues?q=payment+failed&status=all')
  })

  it('chooses the project and stage, the appearance, and signs out', async () => {
    const { signOut } = await open()
    const scope = useScopeStore()
    await type('orders')
    const choice = [...document.body.querySelectorAll<HTMLElement>('[role="option"]')].find((o) =>
      o.textContent.includes('Project orders-backend'),
    )!
    choice.dispatchEvent(new MouseEvent('mousemove', { bubbles: true }))
    choice.click()
    await flushPromises()
    expect(scope.project).toBe(3)

    await type('staging')
    expect(options()).toContain('orders-backend: stage staging')
    await press('Enter')
    expect(scope.environment).toBe(2)

    await type('appearance dark')
    await press('Enter')
    expect(themeChoice.value).toBe('dark')

    await type('sign out')
    await press('Enter')
    expect(signOut).toHaveBeenCalled()
  })
})
