import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import { useShortcuts } from './shortcuts'

afterEach(() => {
  document.body.innerHTML = ''
})

function keys(allowed = true) {
  const actions = { palette: vi.fn(), go: vi.fn(), allowed: vi.fn(() => allowed) }
  const wrapper = mount(
    defineComponent({
      setup: () => {
        useShortcuts(actions)
        return () => h('div')
      },
    }),
    { attachTo: document.body },
  )
  return { actions, wrapper }
}

function press(key: string, init: KeyboardEventInit = {}, target: EventTarget = window): void {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }))
}

describe('the keyboard', () => {
  it('opens the command menu with ⌘K or Ctrl+K', () => {
    const { actions, wrapper } = keys()
    press('k', { metaKey: true })
    press('K', { ctrlKey: true })
    expect(actions.palette).toHaveBeenCalledTimes(2)
    wrapper.unmount()
    press('k', { metaKey: true })
    expect(actions.palette).toHaveBeenCalledTimes(2)
  })

  it('goes to a page with g and its letter', () => {
    const { actions } = keys()
    press('g')
    press('l')
    expect(actions.go).toHaveBeenCalledWith('/logs')
    press('l')
    expect(actions.go).toHaveBeenCalledTimes(1)
    press('g')
    press('x')
    expect(actions.go).toHaveBeenCalledTimes(1)
  })

  it('leaves out the pages one may not open', () => {
    const { actions } = keys(false)
    press('g')
    press('a')
    expect(actions.go).not.toHaveBeenCalled()
  })

  it('focuses the page’s search with /, or opens the menu without one', () => {
    const { actions } = keys()
    press('/')
    expect(actions.palette).toHaveBeenCalledTimes(1)
    const search = document.createElement('input')
    search.setAttribute('data-search', '')
    document.body.append(search)
    press('/')
    expect(document.activeElement).toBe(search)
    expect(actions.palette).toHaveBeenCalledTimes(1)
  })

  it('leaves the keys typed into a field alone', () => {
    const { actions } = keys()
    const field = document.createElement('textarea')
    document.body.append(field)
    press('g', {}, field)
    press('l', {}, field)
    press('/', {}, field)
    press('g', { altKey: true })
    press('l')
    expect(actions.go).not.toHaveBeenCalled()
    expect(actions.palette).not.toHaveBeenCalled()
  })
})
