import type { LocationQuery, LocationQueryRaw } from 'vue-router'

/**
 * List pages whose filters are kept in this browser: opened again without
 * any (from the menu, a back link), they come back as they were left. The
 * named parameters are links into the page, not choices, and are not kept.
 */
export const REMEMBERED: Record<string, readonly string[]> = {
  '/issues': ['page'],
  '/logs': ['trace', 'before'],
  '/performance': ['page'],
  '/monitors': [],
}

const key = (path: string) => `logdock.filters${path}`

export function remember(path: string, query: LocationQuery): void {
  const skipped = REMEMBERED[path]
  if (!skipped) return
  const kept = Object.fromEntries(Object.entries(query).filter(([name]) => !skipped.includes(name)))
  try {
    localStorage.setItem(key(path), JSON.stringify(kept))
  } catch {
    // Storage unavailable: nothing is kept.
  }
}

/** The filters last used on `path`; `null` when none were chosen. */
export function remembered(path: string): LocationQueryRaw | null {
  if (!REMEMBERED[path]) return null
  try {
    const value = JSON.parse(localStorage.getItem(key(path)) ?? 'null') as unknown
    if (value && typeof value === 'object' && Object.keys(value).length > 0) {
      return value as LocationQueryRaw
    }
  } catch {
    // Unreadable: start without.
  }
  return null
}
