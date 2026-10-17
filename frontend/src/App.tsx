import { StartupGate } from '@/features/startup/startup-gate'
import { createDesktopClient } from '@/lib/mailx-client'
import './App.css'

const client = createDesktopClient()

function App() {
  return <StartupGate client={client} />
}

export default App
