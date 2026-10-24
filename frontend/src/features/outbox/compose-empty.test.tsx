import { render, screen } from '@/test/render'
import { describe, expect, it } from 'vitest'
import { DraftPlaceholder, OutboxPlaceholder } from './compose-details'
import { DraftList, OutboxList } from './compose-lists'

// `Empty` is flex-1: it only centres when the pane around it is a full-height flex column.
function fillsPane(node: HTMLElement) {
  const pane = node.closest('[data-slot=list-empty], [data-slot=detail-empty]')
  expect(pane).not.toBeNull()
  expect(pane).toHaveClass('flex', 'h-full', 'flex-col')
  expect(pane).toContainElement(node.closest('[data-slot=empty]'))
}

describe('empty states fill their pane', () => {
  it('keeps the 草稿 title above a centred empty state', () => {
    render(<DraftList drafts={[]} loading={false} error={null} accounts={[]} selectedId={null} onSelect={() => undefined} />)
    expect(screen.getByRole('heading', { name: '草稿' })).toBeInTheDocument()
    fillsPane(screen.getByText('没有草稿'))
  })

  it('keeps the 发件箱 title above a centred empty state', () => {
    render(<OutboxList items={[]} loading={false} error={null} accounts={[]} selectedId={null} onSelect={() => undefined} />)
    expect(screen.getByRole('heading', { name: '发件箱' })).toBeInTheDocument()
    fillsPane(screen.getByText('发件箱是空的'))
  })

  it('keeps the title above a load error', () => {
    render(<DraftList drafts={undefined} loading={false} error="坏了" accounts={[]} selectedId={null} onSelect={() => undefined} />)
    expect(screen.getByRole('heading', { name: '草稿' })).toBeInTheDocument()
    fillsPane(screen.getByText('无法加载草稿'))
  })

  it('centres the detail placeholders', () => {
    render(<><DraftPlaceholder /><OutboxPlaceholder /></>)
    fillsPane(screen.getByText('未选择草稿'))
    fillsPane(screen.getByText('未选择邮件'))
  })
})
