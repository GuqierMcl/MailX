import { AccountsPage } from '@/features/accounts/accounts-page'
import { createDesktopClient } from '@/lib/mailx-client'
import './App.css'

const client = createDesktopClient()

function App() {
  return <AccountsPage client={client} />
}

export default App
