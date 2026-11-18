import { describe, expect, it } from 'vitest'

import { ApiProblem } from '@/shared/api/client'

import de from './de'
import en from './en'
import { detectLocale, i18n, problemMessage, setLocale } from '.'

/** Every key path of a message tree, e.g. `common.save`. */
function keys(tree: object, prefix = ''): string[] {
  return Object.entries(tree).flatMap(([key, value]) =>
    typeof value === 'object' && value !== null
      ? keys(value as object, `${prefix}${key}.`)
      : [`${prefix}${key}`],
  )
}

describe('the translations', () => {
  it('have the same keys in every language', () => {
    expect(keys(de).sort()).toEqual(keys(en).sort())
  })

  it('leave no text empty', () => {
    const empty = (tree: object): string[] =>
      keys(tree).filter((path) => {
        const value = path
          .split('.')
          .reduce<unknown>((node, key) => (node as Record<string, unknown>)[key], tree)
        return typeof value === 'string' && value.trim() === ''
      })
    expect(empty(en)).toEqual([])
    expect(empty(de)).toEqual([])
  })

  it('pick the language from the choice, then the browser', () => {
    expect(detectLocale('de', ['en-US'])).toBe('de')
    expect(detectLocale(null, ['de-AT', 'en'])).toBe('de')
    expect(detectLocale(null, ['fr-FR'])).toBe('en')
    expect(detectLocale('xx', [])).toBe('en')
  })

  it('word problems in the chosen language', () => {
    setLocale('de')
    expect(i18n.global.locale.value).toBe('de')
    expect(problemMessage(new ApiProblem(0, 'network'))).toBe('Logdock ist nicht erreichbar.')
    setLocale('en')
    expect(problemMessage(new ApiProblem(422, 'invalid_input', 'Name too long'))).toBe(
      'Name too long',
    )
    expect(problemMessage(new Error('x'))).toBe('Something went wrong.')
  })
})
