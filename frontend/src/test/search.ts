import userEvent from '@testing-library/user-event'
import { screen } from '@testing-library/react'

/** Applies a search to the list the way a user does: open the palette, type, press Enter. */
export async function searchFor(text: string) {
  await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
  await userEvent.type(await screen.findByRole('combobox'), `${text}{Enter}`)
}
