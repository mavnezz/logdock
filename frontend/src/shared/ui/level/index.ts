export { default as LevelBadge } from './LevelBadge.vue'

/** The four colours of the levels: trace with debug, fatal with error. */
export type LevelGroup = 'debug' | 'info' | 'warn' | 'error'

/** The colour group of a log level or an error event's level. */
export function levelGroup(level: string): LevelGroup {
  if (level === 'trace' || level === 'debug') return 'debug'
  if (level === 'warn' || level === 'warning') return 'warn'
  if (level === 'error' || level === 'fatal') return 'error'
  return 'info'
}
