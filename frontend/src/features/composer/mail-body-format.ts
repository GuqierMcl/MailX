import { t } from '@/i18n'
/** Storage names shared by the editor, toolbar and browser preview client. */
export const MAIL_COLORS = [
  { name: 'gray', get label() { return t('composer:color.gray') } },
  { name: 'red', get label() { return t('composer:color.red') } },
  { name: 'orange', get label() { return t('composer:color.orange') } },
  { name: 'yellow', get label() { return t('composer:color.yellow') } },
  { name: 'green', get label() { return t('composer:color.green') } },
  { name: 'blue', get label() { return t('composer:color.blue') } },
  { name: 'purple', get label() { return t('composer:color.purple') } },
  { name: 'pink', get label() { return t('composer:color.pink') } },
] as const

export const MAIL_HIGHLIGHTS = [
  { name: 'gray', get label() { return t('composer:color.gray') } },
  { name: 'red', get label() { return t('composer:color.red') } },
  { name: 'yellow', get label() { return t('composer:color.yellow') } },
  { name: 'green', get label() { return t('composer:color.green') } },
  { name: 'blue', get label() { return t('composer:color.blue') } },
] as const

export type MailColor = (typeof MAIL_COLORS)[number]['name']
export type MailHighlight = (typeof MAIL_HIGHLIGHTS)[number]['name']
export type MailTextAlign = 'left' | 'center' | 'right'
export type MailStoredTextAlign = Exclude<MailTextAlign, 'left'>

export function isMailColor(value: unknown): value is MailColor {
  return MAIL_COLORS.some((color) => color.name === value)
}

export function isMailHighlight(value: unknown): value is MailHighlight {
  return MAIL_HIGHLIGHTS.some((color) => color.name === value)
}

/** Left alignment is represented by the absence of a stored attribute. */
export function isMailTextAlign(value: unknown): value is MailStoredTextAlign {
  return value === 'center' || value === 'right'
}
