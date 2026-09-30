import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { ContextMenuProvider } from './components/neru/ContextMenu'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ContextMenuProvider>
      <App />
    </ContextMenuProvider>
  </StrictMode>,
)
