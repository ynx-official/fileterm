import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import type { WorkspaceTab } from '@termdock/core'
import { t } from '../../i18n'
import { AppIcon } from '../common/AppIcon'

type DockPanel = 'history' | 'options' | null

type HistoryEntry = {
  command: string
  createdAt: number
}

type DockPreferences = {
  clearAfterSend: boolean
}

const HISTORY_LIMIT = 40
const DEFAULT_PREFERENCES: DockPreferences = {
  clearAfterSend: true
}

function historyStorageKey(profileId: string) {
  return `termdock:terminal-dock:history:${profileId}`
}

function preferencesStorageKey(profileId: string) {
  return `termdock:terminal-dock:preferences:${profileId}`
}

function readStoredJson<T>(key: string, fallback: T) {
  try {
    const raw = window.localStorage.getItem(key)
    if (!raw) {
      return fallback
    }
    return JSON.parse(raw) as T
  } catch {
    return fallback
  }
}

function normalizePreferences(value: Partial<DockPreferences> | null | undefined): DockPreferences {
  return {
    clearAfterSend: value?.clearAfterSend ?? DEFAULT_PREFERENCES.clearAfterSend
  }
}

function isHiddenPath(path: string) {
  return path.split('/').some((segment) => segment.startsWith('.') && segment.length > 1)
}

function formatHistoryTime(timestamp: number) {
  const date = new Date(timestamp)
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export function TerminalDock({
  activeTab,
  connected,
  remotePath,
  filePanelHeight,
  setFilePanelHeight
}: {
  activeTab: WorkspaceTab
  connected: boolean
  remotePath: string
  filePanelHeight?: number
  setFilePanelHeight?: (height: number | ((prev: number) => number)) => void
}) {
  const [command, setCommand] = useState('')
  const [panel, setPanel] = useState<DockPanel>(null)
  const [history, setHistory] = useState<HistoryEntry[]>(() =>
    readStoredJson<HistoryEntry[]>(historyStorageKey(activeTab.profileId), [])
  )
  const [preferences, setPreferences] = useState<DockPreferences>(() =>
    normalizePreferences(
      readStoredJson<Partial<DockPreferences> | null>(preferencesStorageKey(activeTab.profileId), DEFAULT_PREFERENCES)
    )
  )
  const inputRef = useRef<HTMLInputElement | null>(null)
  const rootRef = useRef<HTMLElement | null>(null)

  const [lastFilePanelHeight, setLastFilePanelHeight] = useState(218)

  const handleToggleConnection = async () => {
    if (!window.termdock) return
    if (connected) {
      await window.termdock.disconnectTab(activeTab.id)
    } else {
      await window.termdock.reconnectTab(activeTab.id)
    }
  }

  const handleToggleFilePanel = () => {
    if (!setFilePanelHeight) return
    if (filePanelHeight === 0) {
      setFilePanelHeight(lastFilePanelHeight)
    } else {
      if (filePanelHeight) {
        setLastFilePanelHeight(filePanelHeight)
      }
      setFilePanelHeight(0)
    }
  }

  const [historySearch, setHistorySearch] = useState('')
  const [activeHistoryIndex, setActiveHistoryIndex] = useState(0)
  const [activeTokenIndex, setActiveTokenIndex] = useState(0)
  const historySearchInputRef = useRef<HTMLInputElement | null>(null)

  const filteredHistory = useMemo(() => {
    if (!historySearch.trim()) {
      return history
    }
    const q = historySearch.toLowerCase()
    return history.filter((entry) => entry.command.toLowerCase().includes(q))
  }, [history, historySearch])

  useEffect(() => {
    setActiveHistoryIndex((prev) => {
      if (filteredHistory.length === 0) return 0
      return Math.min(prev, filteredHistory.length - 1)
    })
  }, [filteredHistory.length])

  useEffect(() => {
    setActiveTokenIndex(0)
  }, [activeHistoryIndex])

  useEffect(() => {
    if (panel === 'history') {
      setHistorySearch('')
      setActiveHistoryIndex(0)
      setActiveTokenIndex(0)
      historySearchInputRef.current?.focus()
      document.body.setAttribute('data-history-open', 'true')
    } else {
      document.body.removeAttribute('data-history-open')
    }
    return () => {
      document.body.removeAttribute('data-history-open')
    }
  }, [panel])

  useEffect(() => {
    let lastCtrlPress = 0
    const isEventInTerminalZone = (target: EventTarget | null) => {
      if (!(target instanceof Node)) {
        return false
      }

      const terminalArea = rootRef.current?.parentElement
      return Boolean(terminalArea && terminalArea.contains(target))
    }

    const handleGlobalKeyDown = (event: globalThis.KeyboardEvent) => {
      const eventInTerminalZone = isEventInTerminalZone(event.target)
      if (!eventInTerminalZone && panel !== 'history') {
        return
      }

      if (event.key === 'Control' || event.key === 'Meta') {
        const now = Date.now()
        if (!event.repeat && now - lastCtrlPress < 400) {
          event.preventDefault()
          if (document.activeElement === inputRef.current || document.activeElement === historySearchInputRef.current) {
            inputRef.current?.blur()
            historySearchInputRef.current?.blur()
            window.dispatchEvent(new CustomEvent('termdock:focus-terminal'))
          } else {
            if (panel === 'history') {
              historySearchInputRef.current?.focus()
            } else {
              inputRef.current?.focus()
            }
          }
        }
        lastCtrlPress = now
        return
      }

      if (event.key === 'Alt' && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.repeat) {
        event.preventDefault()
        setPanel((prev) => {
          const next = prev === 'history' ? null : 'history'
          if (next !== 'history') {
            window.dispatchEvent(new CustomEvent('termdock:focus-terminal'))
          }
          return next
        })
        return
      }

      if (panel === 'history') {
        if (event.key === 'Escape') {
          event.preventDefault()
          setPanel(null)
          window.dispatchEvent(new CustomEvent('termdock:focus-terminal'))
          return
        }

        if (event.key === 'ArrowUp') {
          event.preventDefault()
          if (filteredHistory.length > 0) {
            setActiveHistoryIndex((prev) => (prev - 1 + filteredHistory.length) % filteredHistory.length)
          }
          return
        }

        if (event.key === 'ArrowDown') {
          event.preventDefault()
          if (filteredHistory.length > 0) {
            setActiveHistoryIndex((prev) => (prev + 1) % filteredHistory.length)
          }
          return
        }

        if (event.key === 'ArrowLeft') {
          event.preventDefault()
          const targetItem = filteredHistory[activeHistoryIndex]
          if (targetItem) {
            const tokens = targetItem.command.split(/\s+/).filter(Boolean)
            if (tokens.length > 0) {
              setActiveTokenIndex((prev) => (prev - 1 + tokens.length) % tokens.length)
            }
          }
          return
        }

        if (event.key === 'ArrowRight') {
          event.preventDefault()
          const targetItem = filteredHistory[activeHistoryIndex]
          if (targetItem) {
            const tokens = targetItem.command.split(/\s+/).filter(Boolean)
            if (tokens.length > 0) {
              setActiveTokenIndex((prev) => (prev + 1) % tokens.length)
            }
          }
          return
        }

        if (event.key === 'Enter') {
          if (event.isComposing) {
            return
          }
          event.preventDefault()
          const targetItem = filteredHistory[activeHistoryIndex]
          if (targetItem) {
            const tokens = targetItem.command.split(/\s+/).filter(Boolean)
            const selectedToken = tokens[activeTokenIndex]
            if (selectedToken) {
              setCommand((prev) => {
                const trimmed = prev.trim()
                return trimmed ? `${trimmed} ${selectedToken}` : selectedToken
              })
              setPanel(null)
              window.dispatchEvent(new CustomEvent('termdock:focus-terminal'))
            }
          }
          return
        }
      }
    }
    window.addEventListener('keydown', handleGlobalKeyDown)
    return () => window.removeEventListener('keydown', handleGlobalKeyDown)
  }, [panel, filteredHistory, activeHistoryIndex, activeTokenIndex])

  const updatePreferences = (updater: (prev: DockPreferences) => DockPreferences) => {
    setPreferences((prev) => {
      const next = updater(prev)
      window.localStorage.setItem(preferencesStorageKey(activeTab.profileId), JSON.stringify(next))
      return next
    })
  }

  const canSend = connected && activeTab.sessionType === 'ssh' && command.trim().length > 0

  const sendCommand = async (nextCommand: string) => {
    const trimmed = nextCommand.trim()
    if (!trimmed || activeTab.sessionType !== 'ssh' || !connected || !window.termdock?.writeTerminal) {
      return
    }

    await window.termdock.writeTerminal(activeTab.id, `${trimmed}\r`)

    const now = Date.now()
    setHistory((prev) => {
      const deduped = prev.filter((entry) => entry.command !== trimmed)
      const next = [{ command: trimmed, createdAt: now }, ...deduped].slice(0, HISTORY_LIMIT)
      window.localStorage.setItem(historyStorageKey(activeTab.profileId), JSON.stringify(next))
      return next
    })

    if (preferences.clearAfterSend) {
      setCommand('')
    }

    setPanel(null)
    window.requestAnimationFrame(() => inputRef.current?.focus())
  }

  const handleInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape') {
      if (panel) {
        event.preventDefault()
        setPanel(null)
        return
      }
      if (command) {
        event.preventDefault()
        setCommand('')
      }
      return
    }

    if (event.key === 'Enter' && !event.shiftKey) {
      if (event.nativeEvent.isComposing) {
        return
      }
      event.preventDefault()
      if (canSend) {
        void sendCommand(command)
      }
    }
  }

  const handleHistoryAction = async (cmd: string, actionIndex: number) => {
    if (actionIndex === 0) {
      setPanel(null)
      await sendCommand(cmd)
    } else if (actionIndex === 1) {
      setCommand(cmd)
      setPanel(null)
      window.requestAnimationFrame(() => inputRef.current?.focus())
    } else if (actionIndex === 2) {
      setHistory((prev) => {
        const next = prev.filter((entry) => entry.command !== cmd)
        window.localStorage.setItem(historyStorageKey(activeTab.profileId), JSON.stringify(next))
        return next
      })
    }
  }

  const activeItemRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (panel === 'history' && activeItemRef.current) {
      activeItemRef.current.scrollIntoView({ block: 'nearest' })
    }
  }, [activeHistoryIndex, panel])

  const renderHistoryPanel = () => (
    <div className="terminal-dock-panel terminal-dock-history">
      <div className="terminal-dock-history-list">
        {filteredHistory.length ? filteredHistory.map((entry, index) => {
          const isActive = activeHistoryIndex === index
          const tokens = entry.command.split(/\s+/).filter(Boolean)
          return (
            <div
              key={`${entry.createdAt}-${entry.command}`}
              ref={isActive ? activeItemRef : undefined}
              className={`terminal-dock-history-wrapper ${isActive ? 'is-active' : ''}`}
              onClick={() => {
                setActiveHistoryIndex(index)
                setActiveTokenIndex(0)
              }}
              onDoubleClick={() => {
                void handleHistoryAction(entry.command, 0)
              }}
            >
              <div className="terminal-dock-history-left">
                <code>
                  {isActive ? (
                    tokens.map((token, tokenIdx) => (
                      <span
                        key={tokenIdx}
                        className={`history-token ${activeTokenIndex === tokenIdx ? 'is-selected' : ''}`}
                        onClick={(e) => {
                          e.stopPropagation()
                          setCommand((prev) => {
                            const trimmed = prev.trim()
                            return trimmed ? `${trimmed} ${token}` : token
                          })
                          setPanel(null)
                          window.dispatchEvent(new CustomEvent('termdock:focus-terminal'))
                        }}
                      >
                        {token}
                      </span>
                    ))
                  ) : (
                    entry.command
                  )}
                </code>
              </div>
              <div className="terminal-dock-history-right">
                <span className="terminal-dock-history-time">
                  {formatHistoryTime(entry.createdAt)}
                </span>
                <div className="terminal-dock-history-actions">
                  <button
                    className="terminal-dock-action-btn btn-play"
                    type="button"
                    title={t.send}
                    onClick={(e) => {
                      e.stopPropagation()
                      void handleHistoryAction(entry.command, 0)
                    }}
                  >
                    <AppIcon name="play" />
                  </button>
                  <button
                    className="terminal-dock-action-btn btn-copy"
                    type="button"
                    title={t.copy}
                    onClick={(e) => {
                      e.stopPropagation()
                      void handleHistoryAction(entry.command, 1)
                    }}
                  >
                    <AppIcon name="copy" />
                  </button>
                  <button
                    className="terminal-dock-action-btn btn-delete"
                    type="button"
                    title={t.clear}
                    onClick={(e) => {
                      e.stopPropagation()
                      void handleHistoryAction(entry.command, 2)
                    }}
                  >
                    <span className="close-icon">✖</span>
                  </button>
                </div>
              </div>
            </div>
          )
        }) : (
          <div className="terminal-dock-empty">{t.terminalDockHistoryEmpty}</div>
        )}
      </div>
      <div className="terminal-dock-history-footer">
        <span className="terminal-dock-history-hint">{t.terminalDockHistoryInsertHint}</span>
        <button className="terminal-dock-clear-btn" type="button" onClick={() => {
          setHistory([])
          window.localStorage.setItem(historyStorageKey(activeTab.profileId), JSON.stringify([]))
        }}>
          {t.terminalDockClearList}
        </button>
      </div>
      <div className="terminal-dock-history-search-wrapper">
        <input
          ref={historySearchInputRef}
          className="terminal-dock-history-search"
          placeholder={t.terminalDockHistorySearchPlaceholder}
          type="text"
          value={historySearch}
          onChange={(e) => setHistorySearch(e.target.value)}
        />
      </div>
    </div>
  )

  const renderOptionsPanel = () => (
    <div className="terminal-dock-panel terminal-dock-options">
      <label className="terminal-dock-option-row">
        <input
          checked={preferences.clearAfterSend}
          type="checkbox"
          onChange={(event) => updatePreferences((prev) => ({ ...prev, clearAfterSend: event.currentTarget.checked }))}
        />
        <span>{t.terminalDockClearAfterSend}</span>
      </label>
    </div>
  )

  const isMac = window.termdock?.platform === 'darwin'
  const placeholderText = isMac ? t.terminalDockPlaceholderMac : t.terminalDockPlaceholderWin

  return (
    <section ref={rootRef} className="terminal-dock">
      {panel === 'history' ? renderHistoryPanel() : null}
      {panel === 'options' ? renderOptionsPanel() : null}
      {null}
      <div className="terminal-dock-bar">
        <label className="terminal-dock-input-shell">
          <input
            ref={inputRef}
            disabled={activeTab.sessionType !== 'ssh'}
            placeholder={placeholderText}
            type="text"
            value={command}
            onChange={(event) => setCommand(event.currentTarget.value)}
            onKeyDown={handleInputKeyDown}
          />
        </label>
        <div className="terminal-dock-actions">
          <button
            aria-pressed={panel === 'history'}
            className={panel === 'history' ? 'is-active' : undefined}
            type="button"
            onClick={() => setPanel((prev) => prev === 'history' ? null : 'history')}
          >
            {t.history}
          </button>
          <button
            aria-pressed={panel === 'options'}
            className={panel === 'options' ? 'is-active' : undefined}
            type="button"
            onClick={() => setPanel((prev) => prev === 'options' ? null : 'options')}
          >
            {t.options}
          </button>
          <button
            className={`terminal-dock-icon-btn terminal-dock-connection ${connected ? 'is-connected' : 'is-disconnected'}`}
            type="button"
            title={connected ? t.terminalDockDisconnect : t.terminalDockReconnect}
            onClick={handleToggleConnection}
          >
            <AppIcon name="flash" />
          </button>
          <button
            className="terminal-dock-icon-btn"
            type="button"
            title={t.copy}
            onClick={() => window.dispatchEvent(new CustomEvent('termdock:terminal-copy'))}
          >
            <AppIcon name="copy" />
          </button>
          <button
            className="terminal-dock-icon-btn"
            type="button"
            title={t.paste}
            onClick={() => window.dispatchEvent(new CustomEvent('termdock:terminal-paste'))}
          >
            <AppIcon name="paste" />
          </button>
          <button
            className="terminal-dock-icon-btn"
            type="button"
            title={t.find}
            onClick={() => window.dispatchEvent(new CustomEvent('termdock:terminal-find'))}
          >
            <AppIcon name="search" />
          </button>
          {setFilePanelHeight ? (
            <button
              className="terminal-dock-icon-btn"
              type="button"
              title={filePanelHeight === 0 ? t.terminalDockShowFilePanel : t.terminalDockHideFilePanel}
              onClick={handleToggleFilePanel}
            >
              <AppIcon name={filePanelHeight === 0 ? 'chevron-up' : 'chevron-down'} />
            </button>
          ) : null}
        </div>
      </div>
    </section>
  )
}
