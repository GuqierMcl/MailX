import { screen } from '@/test/render'
import userEvent from '@testing-library/user-event'

/** Account folder trees start collapsed; open one (if it is not open already) so its folders can be queried. */
export async function expandAccount(name: string) {
  const row = await screen.findByRole('button', { name })
  if (row.getAttribute('aria-expanded') !== 'true') await userEvent.click(row)
}
