import { t } from '@/i18n'

/** "刚刚", "5 分钟前", "3 小时前", "昨天", then a date. */
export function relativeTime(value: string, language: string, now = Date.now()) {
  const seconds = (Date.parse(value) - now) / 1000
  const elapsed = -seconds
  if (elapsed < 60) return t('now:justNow')
  const format = new Intl.RelativeTimeFormat(language, { numeric: 'auto' })
  if (elapsed < 3600) return format.format(Math.round(seconds / 60), 'minute')
  if (elapsed < 86_400) return format.format(Math.round(seconds / 3600), 'hour')
  if (elapsed < 7 * 86_400) return format.format(Math.round(seconds / 86_400), 'day')
  return new Date(value).toLocaleDateString(language, { month: 'numeric', day: 'numeric' })
}
