import type { ImageMetadata } from 'astro'
import type { Locale } from './locale.mjs'

const files = import.meta.glob<ImageMetadata>('../../../docs/assets/screenshots/{zh-CN,en}/{inbox,now,reading,chat,compose}-{light,dark}.webp', { eager: true, import: 'default' })

/** The light and dark versions of one screenshot in one language. */
export function screenshot(locale: Locale, name: 'inbox' | 'now' | 'reading' | 'chat' | 'compose'): { light: ImageMetadata; dark: ImageMetadata } {
  const pick = (scheme: string) => {
    const path = `../../../docs/assets/screenshots/${locale}/${name}-${scheme}.webp`
    const image = files[path]
    if (!image) throw new Error(`Missing screenshot: ${path}`)
    return image
  }
  return { light: pick('light'), dark: pick('dark') }
}
