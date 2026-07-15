import type { BrowserWindow, WebContents } from 'electron'
import type { DetachWorkspaceTabInput, WorkspaceTabPlacement, WorkspaceWindowContext } from '@fileterm/core'
import { resolveWorkspaceTabPlacements } from './workspace-window-placement.js'

const MAIN_WINDOW_ID = 'main'

type DetachedWindowRecord = {
  context: WorkspaceWindowContext & { kind: 'detached-session'; tabId: string }
  window: BrowserWindow
  ready: boolean
  approvedClose: boolean
}

export interface WorkspaceWindowRegistryOptions {
  getMainWindow(): BrowserWindow | null
  listTabIds(): string[]
  createDetachedWindow(context: WorkspaceWindowContext, input: DetachWorkspaceTabInput): BrowserWindow
  claimTabRenderer(tabId: string, sender: WebContents): void
  releaseTabRenderer(tabId: string, sender: WebContents): void
  broadcastPlacements(placements: WorkspaceTabPlacement[]): void
  isQuitting(): boolean
}

export class WorkspaceWindowRegistry {
  private readonly contextsByWebContentsId = new Map<number, WorkspaceWindowContext>()
  private readonly detachedByTabId = new Map<string, DetachedWindowRecord>()
  private readonly options: WorkspaceWindowRegistryOptions
  private nextWindowNumber = 1

  constructor(options: WorkspaceWindowRegistryOptions) {
    this.options = options
  }

  registerMainWindow(window: BrowserWindow) {
    this.contextsByWebContentsId.set(window.webContents.id, {
      windowId: MAIN_WINDOW_ID,
      kind: 'main'
    })
    window.webContents.once('destroyed', () => {
      this.contextsByWebContentsId.delete(window.webContents.id)
    })
  }

  getContext(sender: WebContents): WorkspaceWindowContext {
    return (
      this.contextsByWebContentsId.get(sender.id) ?? {
        windowId: MAIN_WINDOW_ID,
        kind: 'main'
      }
    )
  }

  listPlacements(): WorkspaceTabPlacement[] {
    return resolveWorkspaceTabPlacements(
      this.options.listTabIds(),
      [...this.detachedByTabId.values()].map((record) => ({
        tabId: record.context.tabId,
        ownerWindowId: record.context.windowId,
        ready: record.ready
      })),
      MAIN_WINDOW_ID
    )
  }

  detach(input: DetachWorkspaceTabInput) {
    if (!this.options.listTabIds().includes(input.tabId)) {
      throw new Error(`Tab not found: ${input.tabId}`)
    }

    const existing = this.detachedByTabId.get(input.tabId)
    if (existing && !existing.window.isDestroyed()) {
      this.activateWindow(existing.window)
      return
    }

    const context: WorkspaceWindowContext & { kind: 'detached-session'; tabId: string } = {
      windowId: `detached-${this.nextWindowNumber++}`,
      kind: 'detached-session',
      tabId: input.tabId
    }
    const window = this.options.createDetachedWindow(context, input)
    const record: DetachedWindowRecord = {
      context,
      window,
      ready: false,
      approvedClose: false
    }

    this.detachedByTabId.set(input.tabId, record)
    this.contextsByWebContentsId.set(window.webContents.id, context)

    window.on('close', (event) => {
      if (this.options.isQuitting() || record.approvedClose) {
        return
      }
      event.preventDefault()
      this.attach(input.tabId)
    })

    window.webContents.on('render-process-gone', () => {
      if (!this.options.isQuitting()) {
        this.attach(input.tabId)
      }
    })

    window.on('closed', () => {
      this.contextsByWebContentsId.delete(window.webContents.id)
      if (this.detachedByTabId.get(input.tabId) === record) {
        this.detachedByTabId.delete(input.tabId)
        this.emitPlacements()
      }
    })
  }

  claim(tabId: string, sender: WebContents) {
    const context = this.getContext(sender)
    if (context.kind === 'detached-session' && context.tabId !== tabId) {
      throw new Error(`Window ${context.windowId} cannot claim tab ${tabId}`)
    }

    const detached = this.detachedByTabId.get(tabId)
    if (context.kind === 'main' && detached) {
      throw new Error(`Tab ${tabId} belongs to detached window ${detached.context.windowId}`)
    }

    this.options.claimTabRenderer(tabId, sender)
    if (detached && detached.window.webContents === sender && !detached.ready) {
      detached.ready = true
      this.emitPlacements()
      this.activateWindow(detached.window)
    }
  }

  attach(tabId: string) {
    const record = this.detachedByTabId.get(tabId)
    if (!record) {
      return
    }

    this.options.releaseTabRenderer(tabId, record.window.webContents)
    this.detachedByTabId.delete(tabId)

    const mainWindow = this.options.getMainWindow()
    if (mainWindow && !mainWindow.isDestroyed()) {
      this.options.claimTabRenderer(tabId, mainWindow.webContents)
      this.activateWindow(mainWindow)
    }

    this.emitPlacements()
    record.approvedClose = true
    if (!record.window.isDestroyed()) {
      record.window.close()
    }
  }

  closeTabWindow(tabId: string) {
    const record = this.detachedByTabId.get(tabId)
    if (!record) {
      return
    }

    this.options.releaseTabRenderer(tabId, record.window.webContents)
    this.detachedByTabId.delete(tabId)
    this.emitPlacements()
    record.approvedClose = true
    if (!record.window.isDestroyed()) {
      record.window.close()
    }
  }

  closeAll() {
    for (const record of this.detachedByTabId.values()) {
      record.approvedClose = true
      if (!record.window.isDestroyed()) {
        record.window.close()
      }
    }
    this.detachedByTabId.clear()
  }

  getDetachedWindows(): BrowserWindow[] {
    return [...this.detachedByTabId.values()].map((record) => record.window).filter((window) => !window.isDestroyed())
  }

  private activateWindow(window: BrowserWindow) {
    if (window.isMinimized()) {
      window.restore()
    }
    window.show()
    window.focus()
  }

  private emitPlacements() {
    this.options.broadcastPlacements(this.listPlacements())
  }
}
