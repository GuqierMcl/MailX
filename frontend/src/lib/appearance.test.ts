import { afterEach, describe, expect, it } from 'vitest'
import {
  DEFAULT_APPEARANCE,
  accentForeground,
  applyAppearance,
  cacheAppearance,
  cachedAppearance,
  fontFamilyCss,
  parseFontFamilies,
} from './appearance'

const root = () => document.documentElement

afterEach(() => {
  localStorage.clear()
  root().className = ''
  root().removeAttribute('style')
})

describe('font families', () => {
  it('splits on commas, trims, unquotes and drops empty entries', () => {
    expect(parseFontFamilies(` "SF Pro Text" , 'PingFang SC',, system-ui `)).toEqual(['SF Pro Text', 'PingFang SC', 'system-ui'])
    expect(parseFontFamilies(' , ')).toEqual([])
  })

  it('uses the default stack only when the user list is empty', () => {
    expect(fontFamilyCss('')).toMatch(/^"Inter Variable", "PingFang SC", .*system-ui, sans-serif$/)
    expect(fontFamilyCss(' ,, ')).toBe(fontFamilyCss(''))
  })

  it('keeps the user order, quotes names and leaves generic families bare', () => {
    expect(fontFamilyCss('LXGW WenKai, Georgia, serif')).toBe('"LXGW WenKai", "Georgia", serif')
  })

  it('ends a list without a generic family in sans-serif', () => {
    expect(fontFamilyCss('Noto Sans SC')).toBe('"Noto Sans SC", sans-serif')
  })

  it('never emits a name that could break out of its quotes', () => {
    expect(fontFamilyCss('Inter, a"b, c\\d')).toBe('"Inter", sans-serif')
  })
})

describe('applyAppearance', () => {
  it('applies the theme, accent, font and size, and resets them', () => {
    applyAppearance({ theme: 'dark', accentColor: '#facc15', interfaceFont: 'Georgia', mailFont: '', mailTheme: 'follow', fontSize: 'large' })
    expect(root()).toHaveClass('dark')
    expect(root().style.getPropertyValue('--primary')).toBe('#facc15')
    expect(root().style.getPropertyValue('--primary-foreground')).toBe('#171717')
    expect(root().style.getPropertyValue('--app-font-sans')).toBe('"Georgia", sans-serif')
    expect(root().style.fontSize).toBe('17px')

    applyAppearance(DEFAULT_APPEARANCE)
    expect(root()).not.toHaveClass('dark')
    expect(root().style.getPropertyValue('--primary')).toBe('')
    expect(root().style.fontSize).toBe('')
  })

  it('picks readable text for light and dark accents', () => {
    expect(accentForeground('#1d4ed8')).toBe('#ffffff')
    expect(accentForeground('#fde047')).toBe('#171717')
  })
})

describe('first-paint cache', () => {
  it('round-trips and tolerates corrupt or partial data', () => {
    const appearance = { ...DEFAULT_APPEARANCE, theme: 'light' as const, interfaceFont: 'Georgia' }
    cacheAppearance(appearance)
    expect(cachedAppearance()).toEqual(appearance)

    localStorage.setItem('mailx.appearance', '{"theme":"sepia","fontSize":3}')
    expect(cachedAppearance()).toEqual(DEFAULT_APPEARANCE)
    localStorage.setItem('mailx.appearance', '{oops')
    expect(cachedAppearance()).toEqual(DEFAULT_APPEARANCE)
  })

  it('falls back to the legacy theme key before settings exist', () => {
    localStorage.setItem('mailx.theme', 'dark')
    expect(cachedAppearance().theme).toBe('dark')
  })
})
