import type { NotificationSettings } from '@/generated/accounts'

/** Same as the core's defaults: on, with previews, quiet while MailX is in front. */
export const DEFAULT_NOTIFICATIONS: NotificationSettings = {
  enabled: true,
  showPreview: true,
  whenFocused: false,
  sound: 'system',
  mutedAccounts: [],
}
