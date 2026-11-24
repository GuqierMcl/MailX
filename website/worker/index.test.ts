import { describe, expect, test } from 'bun:test'
import worker from './index'
import * as entry from './index'
import { chooseLocale, cookieLocale, localizedPath, preferredLanguage } from './language'

const request = (headers: Record<string, string> = {}, path = '/') => new Request(`https://mailxapp.com${path}`, { headers })

test('the entry module exports only the handler, as workerd requires', () => {
  expect(Object.keys(entry)).toEqual(['default'])
})

describe('preferredLanguage', () => {
  test('takes the highest weight, and header order on a tie', () => {
    expect(preferredLanguage('en-US,en;q=0.9,zh-CN;q=0.8')).toBe('en-us')
    expect(preferredLanguage('fr;q=0.5, zh-CN;q=0.9')).toBe('zh-cn')
    expect(preferredLanguage('zh-TW, en')).toBe('zh-tw')
  })

  test('ignores wildcards, zero weights and empty headers', () => {
    expect(preferredLanguage('*, en;q=0')).toBeNull()
    expect(preferredLanguage('')).toBeNull()
    expect(preferredLanguage(null)).toBeNull()
  })
})

test('cookieLocale reads only known values', () => {
  expect(cookieLocale('a=1; mailx-lang=en')).toBe('en')
  expect(cookieLocale('mailx-lang=fr')).toBeNull()
  expect(cookieLocale(null)).toBeNull()
})

describe('chooseLocale', () => {
  test('a cookie beats the browser language', () => {
    expect(chooseLocale(request({ Cookie: 'mailx-lang=zh-CN', 'Accept-Language': 'en' }))).toBe('zh-CN')
    expect(chooseLocale(request({ Cookie: 'mailx-lang=en', 'Accept-Language': 'zh-CN' }))).toBe('en')
  })

  test('any Chinese variant stays Chinese, other languages get English', () => {
    expect(chooseLocale(request({ 'Accept-Language': 'zh-HK,en;q=0.5' }))).toBe('zh-CN')
    expect(chooseLocale(request({ 'Accept-Language': 'ja,zh;q=0.8' }))).toBe('en')
    expect(chooseLocale(request({ 'Accept-Language': 'zhx' }))).toBe('en')
  })

  test('no language header stays on the default', () => {
    expect(chooseLocale(request())).toBe('zh-CN')
  })
})

test('localizedPath always stays on this site', () => {
  expect(localizedPath('en', '/')).toBe('/en/')
  expect(localizedPath('zh-CN', '/docs/privacy')).toBe('/zh-CN/docs/privacy/')
  expect(localizedPath('en', '/privacy')).toBe('/en/docs/privacy/')
  expect(localizedPath('en', '//evil.example/')).toBe('/en//evil.example/')
})

describe('fetch', () => {
  const env = { ASSETS: { fetch: async () => new Response('asset') } }
  const location = async (path: string, headers: Record<string, string> = {}) => {
    const response = await worker.fetch(request(headers, path), env)
    expect(response.status).toBe(302)
    expect(response.headers.get('Vary')).toBe('Accept-Language, Cookie')
    expect(response.headers.get('Cache-Control')).toBe('private, no-store')
    return response.headers.get('Location')
  }

  test('sends an address without a language to the visitor’s language', async () => {
    expect(await location('/', { 'Accept-Language': 'zh-CN' })).toBe('/zh-CN/')
    expect(await location('/?ref=x', { 'Accept-Language': 'en-GB' })).toBe('/en/?ref=x')
    expect(await location('/docs/privacy/', { Cookie: 'mailx-lang=en', 'Accept-Language': 'zh' })).toBe('/en/docs/privacy/')
    expect(await location('/download')).toBe('/zh-CN/download/')
  })

  test('leaves other methods to the assets', async () => {
    const response = await worker.fetch(new Request('https://mailxapp.com/', { method: 'POST' }), env)
    expect(response.status).toBe(200)
  })
})
