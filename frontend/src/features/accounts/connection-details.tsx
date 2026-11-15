import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account, RouteConfig } from '@/generated/accounts'
import { t } from '@/i18n'
import { SettingsDisclosure } from '@/features/settings/settings-layout'
import { needsTrust } from './account-status'
import { ProtocolOutcome } from './connection-verification'
import type { VerificationState } from './use-connection-verification'

/** The folded server, credential and route summary; verification outcomes appear under their server. */
export function ConnectionDetails({ account, verification, open, onOpenChange }: {
  account: Account
  verification: VerificationState
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  useTranslation()
  return (
    <SettingsDisclosure title={t('accounts:connectionDetails')} open={open} onOpenChange={onOpenChange}>
      <DetailRow label={t('accounts:incomingServer')} value={server('IMAP', account.imap)}>
        <ProtocolOutcome state={verification} protocol="imap" />
      </DetailRow>
      <DetailRow label={t('accounts:outgoingServer')} value={server('SMTP', account.smtp)}>
        <ProtocolOutcome state={verification} protocol="smtp" />
      </DetailRow>
      <DetailRow label={t('accounts:credentials')} value={credentials(account, needsTrust(account))} />
      <DetailRow label={t('accounts:networkRoute')} value={route(account.route)} />
    </SettingsDisclosure>
  )
}

/** One compact line: muted label, value on the right (wrapping, never cut: these are the details); `children` sit under the label. */
function DetailRow({ label, value, children }: { label: string; value: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-baseline justify-between gap-4 text-xs">
        <span className="shrink-0 text-muted-foreground">{label}</span>
        <span className="min-w-0 text-right [overflow-wrap:anywhere]">{value}</span>
      </div>
      {children}
    </div>
  )
}

function server(protocol: string, connection: Account['imap']) {
  const security = connection.tls === 'implicit' ? 'TLS' : 'STARTTLS'
  return `${protocol} · ${connection.host}:${connection.port} · ${security} · ${connection.username}`
}

function credentials(account: Account, trustNeeded: boolean) {
  if (account.authentication.kind === 'oauth2') return `${t('accounts:signedInThroughBrowser')} · ${t(account.authentication.state === 'connected' ? 'accounts:oauthConnected' : 'accounts:oauthNeedsReauthorization')}`
  const mode = account.authentication.mode === 'shared' ? t('accounts:sharedPassword') : t('accounts:separatePasswords')
  const configured = account.authentication.imap.passwordConfigured && account.authentication.smtp.passwordConfigured
  return `${mode} · ${trustNeeded ? t('accounts:needsConfirmation') : configured ? t('accounts:storedInCredentialStore') : t('accounts:missingPassword')}`
}

function route(value: RouteConfig) {
  if (value.kind === 'direct') return t('accounts:directConnection')
  if (value.kind === 'inherit') return t('accounts:appDefaultDirect')
  const kind = value.proxyKind === 'socks5' ? 'SOCKS5' : 'HTTP CONNECT'
  return t('accounts:proxySummary', { kind, host: value.host, port: value.port, authentication: value.authentication ? ` · ${value.authentication.username}` : '' })
}
