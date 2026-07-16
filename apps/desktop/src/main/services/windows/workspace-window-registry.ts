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
const DEFAULT_DRAG_RECORD_TTL_MS = 30_000

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
  cleanupTimer: NodeJS.Timeout
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
  dragRecordTtlMs?: number
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
    const webContentsId = window.webContents.id
    this.contextsByWebContentsId.set(webContentsId, {
      windowId: MAIN_WINDOW_ID,
      kind: 'main'
    })
    window.webContents.once('destroyed', () => {
      this.contextsByWebContentsId.delete(webContentsId)
      this.clearDragRecordsForWindow(MAIN_WINDOW_ID)
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
    const webContentsId = window.webContents.id
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
    this.contextsByWebContentsId.set(webContentsId, context)

    window.on('close', (event) => {
      if (this.options.isQuitting() || record.approvedClose) {
        return
      }
      event.preventDefault()
      void this.closeWindowTabs(record)
    })

    window.webContents.on('render-process-gone', () => {
      this.clearDragRecordsForWindow(context.windowId)
      if (!this.options.isQuitting() && !record.approvedClose) {
        this.recoverWindowTabsToMain(record)
      }
    })

    window.on('closed', () => {
      this.contextsByWebContentsId.delete(webContentsId)
      this.clearDragRecordsForWindow(context.windowId)
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
      if (
        !target ||
        target.window.isDestroyed() ||
        target.window.webContents.isDestroyed() ||
        !target.ready ||
        target.approvedClose ||
        target.closeInFlight
      ) {
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

  startDrag(input: WorkspaceTabDragInput, sender: WebContents) {
    this.assertTabExists(input.tabId)
    const sourceContext = this.getRegisteredContext(sender)
    if (!sourceContext || sourceContext.windowId !== input.sourceWindowId) {
      throw new Error(`Workspace tab drag source is not registered: ${input.sourceWindowId}`)
    }

    const sourceWindowId = this.ownerWindowIdByTabId.get(input.tabId) ?? MAIN_WINDOW_ID
    if (sourceWindowId !== sourceContext.windowId) {
      throw new Error(`Tab ${input.tabId} is not owned by window ${sourceContext.windowId}`)
    }

    const existing = this.dragRecords.get(input.dragId)
    if (existing) {
      if (existing.tabId === input.tabId && existing.sourceWindowId === sourceContext.windowId) {
        return
      }
      throw new Error(`Workspace tab drag id is already in use: ${input.dragId}`)
    }

    for (const drag of this.dragRecords.values()) {
      if (drag.tabId === input.tabId) {
        this.deleteDragRecord(drag.dragId)
      }
    }

    const drag: TabDragRecord = {
      ...input,
      sourceWindowId: sourceContext.windowId,
      dropped: false,
      cleanupTimer: setTimeout(() => {
        if (this.dragRecords.get(input.dragId) === drag) {
          this.deleteDragRecord(input.dragId)
        }
      }, this.options.dragRecordTtlMs ?? DEFAULT_DRAG_RECORD_TTL_MS)
    }
    drag.cleanupTimer.unref()
    this.dragRecords.set(input.dragId, drag)
  }

  drop(input: DropWorkspaceTabInput, sender: WebContents) {
    const drag = this.dragRecords.get(input.dragId)
    if (!drag || drag.dropped) {
      return
    }
    if (drag.tabId !== input.tabId || drag.sourceWindowId !== input.sourceWindowId) {
      throw new Error(`Unknown workspace tab drag: ${input.dragId}`)
    }

    const targetContext = this.getRegisteredContext(sender)
    if (!targetContext || !this.canAcceptDrop(targetContext, sender)) {
      throw new Error('Target workspace window is not available')
    }

    const currentOwnerWindowId = this.ownerWindowIdByTabId.get(drag.tabId) ?? MAIN_WINDOW_ID
    if (currentOwnerWindowId !== drag.sourceWindowId || !this.options.listTabIds().includes(drag.tabId)) {
      this.deleteDragRecord(input.dragId)
      return
    }

    if (drag.finishTimer) {
      clearTimeout(drag.finishTimer)
      drag.finishTimer = undefined
    }
    drag.dropped = true
    if (input.dropZone === 'workspace' && targetContext.windowId === drag.sourceWindowId) {
      return
    }
    this.move({
      tabId: drag.tabId,
      targetWindowId: targetContext.windowId,
      targetIndex: input.targetIndex
    })
  }

  finishDrag(input: FinishWorkspaceTabDragInput, sender: WebContents) {
    const drag = this.dragRecords.get(input.dragId)
    if (!drag || drag.dropped) {
      return
    }

    const sourceContext = this.getRegisteredContext(sender)
    if (!sourceContext || sourceContext.windowId !== drag.sourceWindowId) {
      return
    }
    if (!input.detachIfUnhandled || !this.canDetachUnhandledDrag(drag)) {
      this.deleteDragRecord(input.dragId)
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
      const currentOwnerWindowId = this.ownerWindowIdByTabId.get(pending.tabId) ?? MAIN_WINDOW_ID
      if (
        currentOwnerWindowId !== pending.sourceWindowId ||
        !this.options.listTabIds().includes(pending.tabId) ||
        !this.canDetachUnhandledDrag(pending)
      ) {
        this.deleteDragRecord(input.dragId)
        return
      }
      this.deleteDragRecord(input.dragId)
      this.detach({
        tabId: pending.tabId,
        ...(input.screenPoint ? { screenPoint: input.screenPoint } : {})
      })
    }, 80)
    drag.finishTimer.unref()
  }

  closeTabWindow(tabId: string) {
    this.clearDragRecordsForTab(tabId)
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
    for (const dragId of [...this.dragRecords.keys()]) {
      this.deleteDragRecord(dragId)
    }
  }

  getDetachedWindows(): BrowserWindow[] {
    return [...this.detachedByWindowId.values()]
      .map((record) => record.window)
      .filter((window) => !window.isDestroyed())
  }

  private getRegisteredContext(sender: WebContents) {
    return this.contextsByWebContentsId.get(sender.id) ?? null
  }

  private canAcceptDrop(context: WorkspaceWindowContext, sender: WebContents) {
    if (this.options.isQuitting() || sender.isDestroyed()) {
      return false
    }
    if (context.kind === 'main') {
      const mainWindow = this.options.getMainWindow()
      return Boolean(mainWindow && !mainWindow.isDestroyed() && mainWindow.webContents.id === sender.id)
    }

    const record = this.detachedByWindowId.get(context.windowId)
    return Boolean(
      record &&
      record.window.webContents.id === sender.id &&
      !record.window.isDestroyed() &&
      !record.window.webContents.isDestroyed() &&
      record.ready &&
      !record.approvedClose &&
      !record.closeInFlight
    )
  }

  private canDetachUnhandledDrag(drag: WorkspaceTabDragInput) {
    if (drag.sourceWindowId === MAIN_WINDOW_ID) {
      return true
    }

    const source = this.detachedByWindowId.get(drag.sourceWindowId)
    return Boolean(source && source.tabIds.length > 1 && source.tabIds.includes(drag.tabId))
  }

  private deleteDragRecord(dragId: string) {
    const drag = this.dragRecords.get(dragId)
    if (!drag) {
      return
    }
    clearTimeout(drag.cleanupTimer)
    if (drag.finishTimer) {
      clearTimeout(drag.finishTimer)
    }
    this.dragRecords.delete(dragId)
  }

  private clearDragRecordsForTab(tabId: string) {
    for (const drag of this.dragRecords.values()) {
      if (drag.tabId === tabId) {
        this.deleteDragRecord(drag.dragId)
      }
    }
  }

  private clearDragRecordsForWindow(windowId: string) {
    for (const drag of this.dragRecords.values()) {
      if (drag.sourceWindowId === windowId) {
        this.deleteDragRecord(drag.dragId)
      }
    }
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
        this.clearDragRecordsForTab(tabId)
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
      record.window.destroy()
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
