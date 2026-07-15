import type { BrowserWindow, WebContents } from 'electron'
import type {
  DetachWorkspaceTabInput,
  DropWorkspaceTabInput,
  FinishWorkspaceTabDragInput,
  MoveWorkspaceTabInput,
  WorkspaceTabDragInput,
  WorkspaceTabPlacement,
  WorkspaceWindowContext
} from '@fileterm/core'
import { resolveWorkspaceTabPlacements } from './workspace-window-placement.js'

const MAIN_WINDOW_ID = 'main'

type DetachedWindowRecord = {
  context: WorkspaceWindowContext & { kind: 'detached-session'; initialTabId: string }
  window: BrowserWindow
  tabIds: string[]
  ready: boolean
  approvedClose: boolean
  closeInFlight: boolean
}

type TabDragRecord = WorkspaceTabDragInput & {
  dropped: boolean
  finishTimer?: NodeJS.Timeout
}

export interface WorkspaceWindowRegistryOptions {
  getMainWindow(): BrowserWindow | null
  listTabIds(): string[]
  createDetachedWindow(context: WorkspaceWindowContext, input: DetachWorkspaceTabInput): BrowserWindow
  claimTabRenderer(tabId: string, sender: WebContents): void
  releaseTabRenderer(tabId: string, sender: WebContents): void
  closeTab(tabId: string): Promise<void>
  broadcastPlacements(placements: WorkspaceTabPlacement[]): void
  isQuitting(): boolean
}

export class WorkspaceWindowRegistry {
  private readonly contextsByWebContentsId = new Map<number, WorkspaceWindowContext>()
  private readonly detachedByWindowId = new Map<string, DetachedWindowRecord>()
  private readonly ownerWindowIdByTabId = new Map<string, string>()
  private readonly dragRecords = new Map<string, TabDragRecord>()
  private mainTabIds: string[] = []
  private readonly options: WorkspaceWindowRegistryOptions
  private nextWindowNumber = 1

  constructor(options: WorkspaceWindowRegistryOptions) {
    this.options = options
    this.mainTabIds = options.listTabIds()
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
    const tabIds = this.options.listTabIds()
    this.mainTabIds = [
      ...this.mainTabIds.filter((tabId) => tabIds.includes(tabId) && !this.ownerWindowIdByTabId.has(tabId)),
      ...tabIds.filter((tabId) => !this.ownerWindowIdByTabId.has(tabId) && !this.mainTabIds.includes(tabId))
    ]
    return resolveWorkspaceTabPlacements(
      tabIds,
      [...this.detachedByWindowId.values()].flatMap((record) =>
        record.tabIds.map((tabId, order) => ({
          tabId,
          ownerWindowId: record.context.windowId,
          ready: record.ready,
          order
        }))
      ),
      MAIN_WINDOW_ID,
      this.mainTabIds
    )
  }

  detach(input: DetachWorkspaceTabInput) {
    this.assertTabExists(input.tabId)

    const ownerWindowId = this.ownerWindowIdByTabId.get(input.tabId)
    if (ownerWindowId) {
      const existing = this.detachedByWindowId.get(ownerWindowId)
      if (existing && !existing.window.isDestroyed() && existing.tabIds.length === 1) {
        this.activateWindow(existing.window)
        return
      }
    }

    const context: WorkspaceWindowContext & { kind: 'detached-session'; initialTabId: string } = {
      windowId: `detached-${this.nextWindowNumber++}`,
      kind: 'detached-session',
      initialTabId: input.tabId
    }
    const window = this.options.createDetachedWindow(context, input)
    const record: DetachedWindowRecord = {
      context,
      window,
      tabIds: [input.tabId],
      ready: false,
      approvedClose: false,
      closeInFlight: false
    }

    this.detachedByWindowId.set(context.windowId, record)
    this.ownerWindowIdByTabId.set(input.tabId, context.windowId)
    this.contextsByWebContentsId.set(window.webContents.id, context)

    window.on('close', (event) => {
      if (this.options.isQuitting() || record.approvedClose) {
        return
      }
      event.preventDefault()
      void this.closeWindowTabs(record)
    })

    window.webContents.on('render-process-gone', () => {
      if (!this.options.isQuitting() && !record.approvedClose) {
        this.recoverWindowTabsToMain(record)
      }
    })

    window.on('closed', () => {
      this.contextsByWebContentsId.delete(window.webContents.id)
      if (this.detachedByWindowId.get(context.windowId) !== record) {
        return
      }
      if (!this.options.isQuitting() && !record.approvedClose) {
        this.recoverWindowTabsToMain(record)
        return
      }
      this.removeDetachedRecord(record)
      this.emitPlacements()
    })
  }

  claim(tabId: string, sender: WebContents) {
    this.assertTabExists(tabId)
    const context = this.getContext(sender)
    const ownerWindowId = this.ownerWindowIdByTabId.get(tabId) ?? MAIN_WINDOW_ID

    if (context.windowId !== ownerWindowId) {
      throw new Error(`Window ${context.windowId} cannot claim tab ${tabId}; owner is ${ownerWindowId}`)
    }

    const detached = context.kind === 'detached-session' ? this.detachedByWindowId.get(context.windowId) : undefined
    if (detached && !detached.tabIds.includes(tabId)) {
      throw new Error(`Window ${context.windowId} does not contain tab ${tabId}`)
    }

    if (detached && !detached.ready) {
      this.mainTabIds = this.mainTabIds.filter((entry) => entry !== tabId)
      for (const source of this.detachedByWindowId.values()) {
        if (source === detached || !source.tabIds.includes(tabId)) {
          continue
        }
        this.options.releaseTabRenderer(tabId, source.window.webContents)
        source.tabIds = source.tabIds.filter((entry) => entry !== tabId)
        if (source.tabIds.length === 0) {
          this.closeEmptyWindow(source)
        }
      }
    }

    this.options.claimTabRenderer(tabId, sender)
    if (detached && !detached.ready) {
      detached.ready = true
      this.emitPlacements()
      this.activateWindow(detached.window)
    }
  }

  attach(tabId: string) {
    this.move({ tabId, targetWindowId: MAIN_WINDOW_ID })
  }

  move(input: MoveWorkspaceTabInput) {
    this.assertTabExists(input.tabId)
    const sourceWindowId = this.ownerWindowIdByTabId.get(input.tabId) ?? MAIN_WINDOW_ID
    const targetWindowId = input.targetWindowId

    if (targetWindowId !== MAIN_WINDOW_ID) {
      const target = this.detachedByWindowId.get(targetWindowId)
      if (!target || target.window.isDestroyed() || !target.ready) {
        throw new Error(`Target workspace window is not available: ${targetWindowId}`)
      }
    }

    if (sourceWindowId === targetWindowId) {
      this.reorderWithinWindow(input.tabId, targetWindowId, input.targetIndex)
      return
    }

    const sourceWindow = this.getWindow(sourceWindowId)
    const targetWindow = this.getWindow(targetWindowId)
    const sourceRecord = this.detachedByWindowId.get(sourceWindowId)
    const targetRecord = this.detachedByWindowId.get(targetWindowId)

    if (sourceWindow && !sourceWindow.isDestroyed()) {
      this.options.releaseTabRenderer(input.tabId, sourceWindow.webContents)
    }
    if (sourceRecord) {
      sourceRecord.tabIds = sourceRecord.tabIds.filter((tabId) => tabId !== input.tabId)
    } else {
      this.mainTabIds = this.mainTabIds.filter((tabId) => tabId !== input.tabId)
    }

    if (targetRecord) {
      const targetIndex = this.normalizeTargetIndex(input.targetIndex, targetRecord.tabIds.length)
      targetRecord.tabIds.splice(targetIndex, 0, input.tabId)
      this.ownerWindowIdByTabId.set(input.tabId, targetWindowId)
    } else {
      const targetIndex = this.normalizeTargetIndex(input.targetIndex, this.mainTabIds.length)
      this.mainTabIds.splice(targetIndex, 0, input.tabId)
      this.ownerWindowIdByTabId.delete(input.tabId)
    }

    if (targetWindow && !targetWindow.isDestroyed()) {
      this.options.claimTabRenderer(input.tabId, targetWindow.webContents)
    }

    this.emitPlacements()
    if (targetWindow && !targetWindow.isDestroyed()) {
      this.activateWindow(targetWindow)
    }
    if (sourceRecord && sourceRecord.tabIds.length === 0) {
      this.closeEmptyWindow(sourceRecord)
    }
  }

  startDrag(input: WorkspaceTabDragInput) {
    this.assertTabExists(input.tabId)
    const sourceWindowId = this.ownerWindowIdByTabId.get(input.tabId) ?? MAIN_WINDOW_ID
    if (sourceWindowId !== input.sourceWindowId) {
      throw new Error(`Tab ${input.tabId} is not owned by window ${input.sourceWindowId}`)
    }
    const existing = this.dragRecords.get(input.dragId)
    if (existing?.finishTimer) {
      clearTimeout(existing.finishTimer)
    }
    this.dragRecords.set(input.dragId, { ...input, dropped: false })
  }

  drop(input: DropWorkspaceTabInput) {
    let drag = this.dragRecords.get(input.dragId)
    if (!drag) {
      const currentOwnerWindowId = this.ownerWindowIdByTabId.get(input.tabId) ?? MAIN_WINDOW_ID
      if (currentOwnerWindowId !== input.sourceWindowId) {
        throw new Error(`Tab ${input.tabId} is not owned by window ${input.sourceWindowId}`)
      }
      drag = {
        dragId: input.dragId,
        tabId: input.tabId,
        sourceWindowId: input.sourceWindowId,
        dropped: false
      }
      this.dragRecords.set(input.dragId, drag)
    }
    if (drag.tabId !== input.tabId) {
      throw new Error(`Unknown workspace tab drag: ${input.dragId}`)
    }
    if (drag.finishTimer) {
      clearTimeout(drag.finishTimer)
    }
    drag.dropped = true
    this.move(input)
    this.dragRecords.delete(input.dragId)
  }

  finishDrag(input: FinishWorkspaceTabDragInput) {
    const drag = this.dragRecords.get(input.dragId)
    if (!drag || drag.dropped) {
      return
    }
    if (!input.detachIfUnhandled) {
      if (drag.finishTimer) {
        clearTimeout(drag.finishTimer)
      }
      this.dragRecords.delete(input.dragId)
      return
    }
    if (drag.finishTimer) {
      clearTimeout(drag.finishTimer)
    }
    drag.finishTimer = setTimeout(() => {
      const pending = this.dragRecords.get(input.dragId)
      if (!pending || pending.dropped) {
        return
      }
      this.dragRecords.delete(input.dragId)
      this.detach({
        tabId: pending.tabId,
        ...(input.screenPoint ? { screenPoint: input.screenPoint } : {})
      })
    }, 80)
  }

  closeTabWindow(tabId: string) {
    const ownerWindowId = this.ownerWindowIdByTabId.get(tabId)
    if (!ownerWindowId) {
      return
    }

    const record = this.detachedByWindowId.get(ownerWindowId)
    this.ownerWindowIdByTabId.delete(tabId)
    if (!record) {
      return
    }

    record.tabIds = record.tabIds.filter((entry) => entry !== tabId)
    this.emitPlacements()
    if (record.tabIds.length === 0) {
      this.closeEmptyWindow(record)
    }
  }

  closeAll() {
    for (const record of this.detachedByWindowId.values()) {
      record.approvedClose = true
      if (!record.window.isDestroyed()) {
        record.window.close()
      }
    }
    this.detachedByWindowId.clear()
    this.ownerWindowIdByTabId.clear()
    for (const drag of this.dragRecords.values()) {
      if (drag.finishTimer) {
        clearTimeout(drag.finishTimer)
      }
    }
    this.dragRecords.clear()
  }

  getDetachedWindows(): BrowserWindow[] {
    return [...this.detachedByWindowId.values()]
      .map((record) => record.window)
      .filter((window) => !window.isDestroyed())
  }

  private assertTabExists(tabId: string) {
    if (!this.options.listTabIds().includes(tabId)) {
      throw new Error(`Tab not found: ${tabId}`)
    }
  }

  private getWindow(windowId: string) {
    if (windowId === MAIN_WINDOW_ID) {
      return this.options.getMainWindow()
    }
    return this.detachedByWindowId.get(windowId)?.window ?? null
  }

  private reorderWithinWindow(tabId: string, windowId: string, requestedIndex?: number) {
    if (windowId === MAIN_WINDOW_ID) {
      const currentIndex = this.mainTabIds.indexOf(tabId)
      if (currentIndex === -1) {
        return
      }
      this.mainTabIds.splice(currentIndex, 1)
      const targetIndex = this.normalizeTargetIndex(requestedIndex, this.mainTabIds.length)
      this.mainTabIds.splice(targetIndex, 0, tabId)
      this.emitPlacements()
      const mainWindow = this.options.getMainWindow()
      if (mainWindow && !mainWindow.isDestroyed()) {
        this.activateWindow(mainWindow)
      }
      return
    }

    const record = this.detachedByWindowId.get(windowId)
    if (!record) {
      return
    }
    const currentIndex = record.tabIds.indexOf(tabId)
    if (currentIndex === -1) {
      return
    }
    record.tabIds.splice(currentIndex, 1)
    const targetIndex = this.normalizeTargetIndex(requestedIndex, record.tabIds.length)
    record.tabIds.splice(targetIndex, 0, tabId)
    this.emitPlacements()
    this.activateWindow(record.window)
  }

  private normalizeTargetIndex(targetIndex: number | undefined, length: number) {
    if (targetIndex === undefined || !Number.isFinite(targetIndex)) {
      return length
    }
    return Math.max(0, Math.min(Math.floor(targetIndex), length))
  }

  private async closeWindowTabs(record: DetachedWindowRecord) {
    if (record.closeInFlight) {
      return
    }
    record.closeInFlight = true
    try {
      for (const tabId of [...record.tabIds]) {
        await this.options.closeTab(tabId)
        this.ownerWindowIdByTabId.delete(tabId)
        record.tabIds = record.tabIds.filter((entry) => entry !== tabId)
        this.emitPlacements()
      }
      record.approvedClose = true
      if (!record.window.isDestroyed()) {
        record.window.close()
      }
    } catch {
      record.approvedClose = false
    } finally {
      record.closeInFlight = false
    }
  }

  private recoverWindowTabsToMain(record: DetachedWindowRecord) {
    if (this.detachedByWindowId.get(record.context.windowId) !== record) {
      return
    }

    const mainWindow = this.options.getMainWindow()
    for (const tabId of [...record.tabIds]) {
      this.options.releaseTabRenderer(tabId, record.window.webContents)
      this.ownerWindowIdByTabId.delete(tabId)
      if (mainWindow && !mainWindow.isDestroyed()) {
        this.options.claimTabRenderer(tabId, mainWindow.webContents)
      }
    }
    record.tabIds = []
    record.approvedClose = true
    this.removeDetachedRecord(record)
    this.emitPlacements()
    if (mainWindow && !mainWindow.isDestroyed()) {
      this.activateWindow(mainWindow)
    }
    if (!record.window.isDestroyed()) {
      record.window.close()
    }
  }

  private closeEmptyWindow(record: DetachedWindowRecord) {
    record.approvedClose = true
    this.removeDetachedRecord(record)
    if (!record.window.isDestroyed()) {
      record.window.close()
    }
  }

  private removeDetachedRecord(record: DetachedWindowRecord) {
    if (this.detachedByWindowId.get(record.context.windowId) === record) {
      this.detachedByWindowId.delete(record.context.windowId)
    }
    for (const tabId of record.tabIds) {
      if (this.ownerWindowIdByTabId.get(tabId) === record.context.windowId) {
        this.ownerWindowIdByTabId.delete(tabId)
      }
    }
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
