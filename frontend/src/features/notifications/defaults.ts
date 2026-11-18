import type { BadgeSettings, NotificationSettings } from '@/generated/accounts'

/** Same as the core's defaults: on, with previews, quiet while MailX is in front. */
export const DEFAULT_NOTIFICATIONS: NotificationSettings = {
  enabled: true,
  showPreview: true,
  whenFocused: false,
  sound: 'system',
  mutedAccounts: [],
}

/** Same as the core's defaults: the app icon counts every account. */
export const DEFAULT_BADGE: BadgeSettings = {
  enabled: true,
  excludedAccounts: [],
}
