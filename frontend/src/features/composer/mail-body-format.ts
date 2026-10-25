/** Storage names shared by the editor, toolbar and browser preview client. */
export const MAIL_COLORS = [
  { name: 'gray', label: '灰色' },
  { name: 'red', label: '红色' },
  { name: 'orange', label: '橙色' },
  { name: 'yellow', label: '黄色' },
  { name: 'green', label: '绿色' },
  { name: 'blue', label: '蓝色' },
  { name: 'purple', label: '紫色' },
  { name: 'pink', label: '粉色' },
] as const

export const MAIL_HIGHLIGHTS = [
  { name: 'gray', label: '灰色' },
  { name: 'red', label: '红色' },
  { name: 'yellow', label: '黄色' },
  { name: 'green', label: '绿色' },
  { name: 'blue', label: '蓝色' },
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
