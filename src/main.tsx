import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import App from './App'
import { CrashScreen } from './components/CrashScreen'
import { Sentry, initErrorReporting } from './lib/telemetry'

initErrorReporting()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Sentry.ErrorBoundary fallback={({ error, eventId }) => <CrashScreen error={error} eventId={eventId} />}>
      <App />
    </Sentry.ErrorBoundary>
  </StrictMode>,
)
