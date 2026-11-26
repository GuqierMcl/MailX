import { t } from '@/i18n'
import { CodeRow } from './codes-view'
import type { NowModule } from './types'

/** Most codes listed, as the overview caps its own list. */
export const OVERVIEW_CODES = 3

/** Codes from the last minutes; they expire, so they come before drafts. */
export const codesModule: NowModule = {
  id: 'codes',
  zone: 'act',
  hideable: true,
  fromOverview: true,
  title: () => t('now:codes'),
  score: ({ data }) => (data.codes.length > 0 ? 320 : 300),
  signature: ({ codes }) => codes.map((code) => code.emailId).join('|'),
  hasContent: ({ codes }) => codes.length > 0,
  render: ({ data, language, minute, verification, actions }) => (
    <ul className="flex flex-col rounded-xl bg-muted/40 p-1">
      {data.codes.map((code) => (
        <CodeRow key={code.emailId} code={code} language={language} now={minute} verification={verification} onOpenEmail={actions.onOpenEmail} />
      ))}
    </ul>
  ),
}
