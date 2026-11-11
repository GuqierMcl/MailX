import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { DragDropProvider } from '@dnd-kit/react'
import { isSortable, useSortable } from '@dnd-kit/react/sortable'
import { Accessibility } from '@dnd-kit/dom'
import { OptimisticSortingPlugin } from '@dnd-kit/dom/sortable'
import { move } from '@dnd-kit/helpers'
import { HugeiconsIcon } from '@hugeicons/react'
import { DragDropVerticalIcon } from '@hugeicons/core-free-icons'
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem } from '@/components/ui/sidebar'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { Account } from '@/generated/accounts'
import { t } from '@/i18n'
import { AccountAvatar } from './account-avatar'
import { providerOf } from './providers'

type Props = {
  accounts: Account[]
  activeId: string | null
  disabled: boolean
  reorderDisabled: boolean
  onSelect: (id: string) => void
  onReorder: (ids: string[]) => Promise<unknown>
}

const controlledSorting: NonNullable<Parameters<typeof useSortable>[0]['plugins']> =
  (defaults) => defaults.filter((plugin) => plugin !== OptimisticSortingPlugin)

/** The library handles pointer/keyboard input, sorting feedback and scrolling. */
export function SortableAccountList({ accounts, activeId, disabled, reorderDisabled, onSelect, onReorder }: Props) {
  useTranslation()
  const reducedMotion = usePrefersReducedMotion()
  const startedWith = useRef<string[]>([])
  const [preview, setPreview] = useState<Account[] | null>(null)
  const displayed = preview ?? accounts
  const instructionsId = useId()
  const latestAccounts = useRef(accounts)
  useEffect(() => { latestAccounts.current = accounts }, [accounts])
  const ids = accounts.map((account) => account.id)
  const accessibility = useMemo<NonNullable<ConstructorParameters<typeof Accessibility>[1]>>(() => ({
    screenReaderInstructions: { draggable: t('accounts:reorderInstructions') },
    announcements: {
      dragstart: ({ operation: { source } }) => source ? t('accounts:reorderStarted', {
        name: latestAccounts.current.find((account) => account.id === source.id)?.displayName ?? '',
        position: isSortable(source) ? source.initialIndex + 1 : 1, count: latestAccounts.current.length,
      }) : undefined,
      dragover: ({ operation: { source } }) => source && isSortable(source) ? t('accounts:reorderMoved', {
        name: latestAccounts.current.find((account) => account.id === source.id)?.displayName ?? '',
        position: source.index + 1, count: latestAccounts.current.length,
      }) : undefined,
      dragend: ({ operation: { source }, canceled }) => source ? t(canceled ? 'accounts:reorderCanceled' : 'accounts:reorderFinished', {
        name: latestAccounts.current.find((account) => account.id === source.id)?.displayName ?? '',
        position: isSortable(source) ? source.index + 1 : 1, count: latestAccounts.current.length,
      }) : undefined,
    },
  }), [])

  return (
    <DragDropProvider
      plugins={(defaults) => defaults.map((plugin) => plugin === Accessibility ? {
        plugin: Accessibility,
        options: accessibility,
      } : plugin)}
      onDragStart={() => { startedWith.current = ids }}
      onDragOver={(event) => setPreview((current) => move(current ?? accounts, event))}
      onDragEnd={(event) => {
        // An account removal or another window's reorder invalidates the drag's
        // indices. Keep that newer list and let the user start again.
        if (event.canceled || startedWith.current.some((id, index) => id !== ids[index]) || startedWith.current.length !== ids.length) {
          setPreview(null)
          return
        }
        const next = (preview ?? accounts).map((account) => account.id)
        if (next.some((id, index) => id !== ids[index])) {
          // Retain the preview until the optimistic settings write settles.
          // A rejection then restores the latest committed order in one render.
          void onReorder(next).then(() => setPreview(null), () => setPreview(null))
        } else setPreview(null)
      }}
    >
      <span id={instructionsId} className="sr-only">{t('accounts:reorderInstructions')}</span>
      <SidebarMenu className="flex-row overflow-x-auto md:flex-col md:overflow-visible" aria-label={t('accounts:emailAccountList')}>
        {displayed.map((account, index) => (
          <SortableAccountRow key={account.id} account={account} index={index} active={account.id === activeId}
            disabled={disabled} reorderDisabled={reorderDisabled || accounts.length < 2} instructionsId={instructionsId} reducedMotion={reducedMotion} onSelect={onSelect} />
        ))}
      </SidebarMenu>
    </DragDropProvider>
  )
}

function SortableAccountRow({ account, index, active, disabled, reorderDisabled, instructionsId, reducedMotion, onSelect }: {
  account: Account; index: number; active: boolean; disabled: boolean; reorderDisabled: boolean; instructionsId: string; reducedMotion: boolean; onSelect: (id: string) => void
}) {
  const { ref, handleRef, isDragging } = useSortable({
    id: account.id, index, disabled: reorderDisabled, transition: reducedMotion ? null : undefined,
    // React owns the preview order so an immediate storage failure can always
    // restore it, including when React batches the settings cache updates.
    plugins: controlledSorting,
  })
  return (
    <SidebarMenuItem ref={ref} className="flex shrink-0 items-center gap-0.5 rounded-md bg-sidebar data-[dragging=true]:z-10 data-[dragging=true]:shadow-md" data-dragging={isDragging}>
      <button ref={handleRef} type="button" disabled={disabled} aria-disabled={reorderDisabled} aria-describedby={instructionsId}
        aria-label={t('accounts:reorderAccount', { name: account.displayName })} aria-roledescription={t('accounts:reorderRoleDescription')}
        className="flex h-8 w-6 shrink-0 touch-none cursor-grab items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-sidebar-ring active:cursor-grabbing aria-disabled:cursor-default aria-disabled:opacity-30 disabled:cursor-default disabled:opacity-30">
        <HugeiconsIcon icon={DragDropVerticalIcon} className="size-4" aria-hidden />
      </button>
      <SidebarMenuButton type="button" disabled={disabled} isActive={active} title={`${account.emailAddress} · ${providerOf(account.providerId).name}`} onClick={() => onSelect(account.id)}>
        <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-5 text-[10px]" />
        <span>{account.displayName}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  )
}
