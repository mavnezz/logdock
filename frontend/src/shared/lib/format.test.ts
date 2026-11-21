import { describe, expect, it } from 'vitest'

import { setLocale } from '@/shared/i18n'

import {
  formatBytes,
  formatDay,
  formatTick,
  formatDateTime,
  formatDuration,
  formatNumber,
  formatRelative,
  formatTime,
} from './format'

describe('formatting', () => {
  it('writes times, counts and sizes', () => {
    setLocale('en')
    expect(formatDateTime(null)).toBe('–')
    expect(formatDateTime('nonsense')).toBe('nonsense')
    expect(formatDateTime('2026-11-18T09:15:02.123456Z')).toMatch(/2026/)
    expect(formatTime('2026-11-18T09:15:02.123Z')).toMatch(/\.123$/)
    expect(formatTime('x')).toBe('x')
    const now = new Date('2026-11-18T10:00:00Z')
    expect(formatRelative('2026-11-18T09:57:00Z', now)).toBe('3 minutes ago')
    expect(formatRelative('2026-11-18T12:00:00Z', now)).toBe('in 2 hours')
    expect(formatRelative('2026-11-16T10:00:00Z', now)).toBe('2 days ago')
    expect(formatRelative('2026-11-18T09:59:58Z', now)).toBe('2 seconds ago')
    expect(formatRelative(undefined)).toBe('–')
    expect(formatNumber(12345)).toBe('12,345')
    expect(formatDay('2026-11-20')).toBe('20 Nov 2026')
    expect(formatDay('soon')).toBe('soon')
    const day = 86_400_000
    expect(formatTick('2026-11-20T09:05:00Z', day)).toMatch(/^\d{2}:05$/)
    expect(formatTick('2026-11-20T09:05:00Z', 7 * day)).toBe('20 Nov')
    expect(formatTick('later', day)).toBe('later')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1536)).toBe('1.5 KiB')
    expect(formatBytes(50 * 1024 ** 3)).toBe('50 GiB')
    expect(formatDuration(0.5)).toBe('0.5 ms')
    expect(formatDuration(850)).toBe('850 ms')
    expect(formatDuration(1234)).toBe('1.2 s')
    expect(formatDuration(120_000)).toBe('2 min')
    setLocale('de')
    expect(formatNumber(12345)).toBe('12.345')
    setLocale('en')
  })
})
