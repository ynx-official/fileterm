import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { ErrorBoundary } from './features/common/ErrorBoundary'
import { createTauriApi } from '../bridge/tauri-api'
import { getCurrentWindow } from '@tauri-apps/api/window'
import './styles/index.css'

const initialWindowMode = new URLSearchParams(window.location.search).get('window') ?? 'main'
document.documentElement.classList.toggle('tauri-standalone-window', initialWindowMode !== 'main')

// Global window dragging handler for Tauri custom titlebars
window.addEventListener('mousedown', (e) => {
  // Only trigger dragging on left-click
  if (e.button !== 0) return

  let target = e.target as HTMLElement | null
  let isDragRegion = false

  while (target) {
    const tagName = target.tagName.toLowerCase()

    // Exclude basic interactive elements
    if (
      tagName === 'button' ||
      tagName === 'input' ||
      tagName === 'textarea' ||
      tagName === 'select' ||
      tagName === 'a'
    ) {
      return
    }

    // Exclude tabs, tab-tools, and other interactive containers
    if (
      target.getAttribute('role') === 'button' ||
      target.getAttribute('contenteditable') === 'true' ||
      target.classList.contains('fs-tab') ||
      target.classList.contains('add-tab') ||
      target.classList.contains('window-tools') ||
      target.classList.contains('window-controls-decorator') ||
      target.closest('.no-drag')
    ) {
      return
    }

    if (target.hasAttribute('data-tauri-drag-region')) {
      isDragRegion = true
      break
    }

    target = target.parentElement
  }

  if (isDragRegion) {
    try {
      getCurrentWindow().startDragging()
    } catch (err) {
      console.error('Failed to start window dragging:', err)
    }
  }
})

window.fileterm = createTauriApi()
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
)
