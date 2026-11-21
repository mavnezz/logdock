// Formatting of times, counts and sizes in the interface's language.

import { i18n } from '@/shared/i18n'

function locale(): string {
  return i18n.global.locale.value === 'de' ? 'de-DE' : 'en-GB'
}

/** Date and time in the viewer's time zone, to the second. */
export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return '–'
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  return date.toLocaleString(locale(), {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

/** A day (`YYYY-MM-DD`, UTC) as the language writes it: `20 Nov 2026`. */
export function formatDay(day: string): string {
  const date = new Date(`${day}T00:00:00Z`)
  if (Number.isNaN(date.getTime())) return day
  return date.toLocaleDateString(locale(), {
    timeZone: 'UTC',
    day: 'numeric',
    month: 'short',
    year: 'numeric',
  })
}

/** Time of day with milliseconds, for log lines. */
export function formatTime(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const time = date.toLocaleTimeString(locale(), {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
  return `${time}.${String(date.getMilliseconds()).padStart(3, '0')}`
}

/**
 * A chart axis's mark: the time of day over a range of a day and a half at
 * most, the date over a longer one.
 */
export function formatTick(iso: string, spanMs: number): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  return spanMs <= 36 * 3_600_000
    ? date.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit', hour12: false })
    : date.toLocaleDateString(locale(), { day: 'numeric', month: 'short' })
}

/** "3 minutes ago", "in 2 hours". */
export function formatRelative(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return '–'
  const date = new Date(iso)
  const seconds = Math.round((date.getTime() - now.getTime()) / 1000)
  const format = new Intl.RelativeTimeFormat(locale(), { numeric: 'auto' })
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['day', 86_400],
    ['hour', 3_600],
    ['minute', 60],
  ]
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return format.format(Math.round(seconds / size), unit)
  }
  return format.format(seconds, 'second')
}

export function formatNumber(value: number): string {
  return value.toLocaleString(locale())
}

/** Bytes as KiB, MiB, GiB. */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1
  return `${value.toLocaleString(locale(), { maximumFractionDigits: digits })} ${units[unit]}`
}

/** Milliseconds as `850 ms`, `1.2 s`, `2 min`. */
export function formatDuration(ms: number): string {
  if (ms < 1) return `${ms.toLocaleString(locale(), { maximumFractionDigits: 2 })} ms`
  if (ms < 1000) return `${Math.round(ms).toLocaleString(locale())} ms`
  if (ms < 60_000) return `${(ms / 1000).toLocaleString(locale(), { maximumFractionDigits: 1 })} s`
  return `${(ms / 60_000).toLocaleString(locale(), { maximumFractionDigits: 1 })} min`
}
