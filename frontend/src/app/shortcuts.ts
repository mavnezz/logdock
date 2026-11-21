import { onBeforeUnmount, onMounted } from 'vue'

import { navigation } from './navigation'

/** How long `g` waits for the key of where to go. */
const CHORD_MS = 1200

function typing(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  )
}

/**
 * The keyboard: ⌘K or Ctrl+K opens the command menu, `/` the page's search
 * (an input marked `data-search`), and `g` then a letter goes to a page.
 * Keys typed into a field stay there.
 */
export function useShortcuts(actions: {
  palette: () => void
  go: (path: string) => void
  allowed: (path: string) => boolean
}): void {
  let chord = 0

  function onKey(event: KeyboardEvent): void {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault()
      actions.palette()
      return
    }
    if (event.defaultPrevented || typing(event.target)) return
    if (event.metaKey || event.ctrlKey || event.altKey) return
    if (event.key === '/') {
      event.preventDefault()
      const field = document.querySelector<HTMLInputElement>('input[data-search]')
      if (field) {
        field.focus()
        field.select()
      } else {
        actions.palette()
      }
      return
    }
    if (chord && Date.now() - chord < CHORD_MS) {
      chord = 0
      const item = navigation.find((i) => i.key === event.key && actions.allowed(i.path))
      if (item) {
        event.preventDefault()
        actions.go(item.path)
      }
      return
    }
    chord = event.key === 'g' ? Date.now() : 0
  }

  onMounted(() => window.addEventListener('keydown', onKey))
  onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
}
