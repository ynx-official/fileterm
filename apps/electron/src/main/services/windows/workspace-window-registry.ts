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
const DRAG_TARGET_HOVER_GRACE_MS = 1_000

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
  hoverTargetWindowId?: string
  hoverTargetAt?: number
  cleanupTimer: NodeJS.Timeout
  finishTimer?: NodeJS.Timeout
}

export interface WorkspaceWindowRegistryOptions {
  getMainWindow(): BrowserWindow | null
  ensureMainWindow(): BrowserWindow
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
  private mainCloseInFlight = false
  private readonly options: WorkspaceWindowRegistryOptions
  private nextWindowNumber = 1

  constructor(options: WorkspaceWindowRegistryOptions) {
    this.options = options
    this.mainTabIds = options.listTabIds()
  }

  registerMainWindow(window: BrowserWindow) {
    const webContentsId = window.webContents.id
    this.mainCloseInFlight = false
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
    const context = this.getRegisteredContext(sender)
    if (!context) {
      throw new Error('Workspace tab claim sender is not registered')
    }
    const ownerWindowId = this.ownerWindowIdByTabId.get(tabId) ?? MAIN_WINDOW_ID

    if (context.windowId !== ownerWindowId) {
      return
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

  placeNewTab(tabId: string, sender: WebContents) {
    this.assertTabExists(tabId)
    const context = this.getRegisteredContext(sender)
    if (!context || !this.canAcceptDrop(context, sender)) {
      throw new Error('Workspace tab placement sender is not registered')
    }

    this.mainTabIds = this.mainTabIds.filter((entry) => entry !== tabId)
    for (const record of this.detachedByWindowId.values()) {
      record.tabIds = record.tabIds.filter((entry) => entry !== tabId)
    }

    if (context.kind === 'main') {
      this.ownerWindowIdByTabId.delete(tabId)
      this.mainTabIds.push(tabId)
    } else {
      const target = this.detachedByWindowId.get(context.windowId)
      if (!target) {
        throw new Error(`Workspace window not found: ${context.windowId}`)
      }
      target.tabIds.push(tabId)
      this.ownerWindowIdByTabId.set(tabId, context.windowId)
    }

    this.emitPlacements()
  }

  attach(tabId: string) {
    this.move({ tabId, targetWindowId: MAIN_WINDOW_ID })
  }

  move(input: MoveWorkspaceTabInput) {
    this.assertTabExists(input.tabId)
    const sourceWindowId = this.ownerWindowIdByTabId.get(input.tabId) ?? MAIN_WINDOW_ID
    const targetWindowId = input.targetWindowId

    if (targetWindowId === MAIN_WINDOW_ID) {
      const mainWindow = this.options.getMainWindow()
      if (this.mainCloseInFlight || !mainWindow || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed()) {
        throw new Error(`Target workspace window is not available: ${targetWindowId}`)
      }
    } else {
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

    if (sourceRecord && sourceRecord.tabIds.length === 0) {
      this.closeEmptyWindow(sourceRecord)
    } else if (sourceWindowId === MAIN_WINDOW_ID && this.mainTabIds.length === 0) {
      this.closeEmptyMainWindow()
    }
    this.emitPlacements()
    if (targetWindow && !targetWindow.isDestroyed()) {
      this.activateWindow(targetWindow)
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

  setDragTarget(active: boolean, sender: WebContents) {
    const targetContext = this.getRegisteredContext(sender)
    if (!targetContext || !this.canAcceptDrop(targetContext, sender)) {
      return
    }

    const now = Date.now()
    for (const drag of this.dragRecords.values()) {
      if (drag.dropped) {
        continue
      }
      if (drag.sourceWindowId === targetContext.windowId) {
        if (active) {
          drag.hoverTargetWindowId = targetContext.windowId
          drag.hoverTargetAt = now
        } else if (drag.hoverTargetWindowId === targetContext.windowId) {
          drag.hoverTargetWindowId = undefined
          drag.hoverTargetAt = undefined
        }
        continue
      }
      if (active) {
        drag.hoverTargetWindowId = targetContext.windowId
        drag.hoverTargetAt = now
      } else if (drag.hoverTargetWindowId === targetContext.windowId) {
        drag.hoverTargetAt = now
      }
    }
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
    const canDetach = input.detachIfUnhandled && this.canDetachUnhandledDrag(drag)
    if (!input.screenPoint && !canDetach) {
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
      if (currentOwnerWindowId !== pending.sourceWindowId || !this.options.listTabIds().includes(pending.tabId)) {
        this.deleteDragRecord(input.dragId)
        return
      }

      const hoveredTargetWindowId = input.screenPoint
        ? this.resolveRecentHoveredDropTarget(pending, input.screenPoint)
        : null
      if (hoveredTargetWindowId === pending.sourceWindowId) {
        this.deleteDragRecord(input.dragId)
        return
      }
      const fallbackTargetWindowId =
        hoveredTargetWindowId ??
        (input.screenPoint ? this.findWorkspaceDropTargetAtPoint(input.screenPoint, pending.sourceWindowId) : null)
      if (fallbackTargetWindowId === pending.sourceWindowId && !canDetach) {
        this.deleteDragRecord(input.dragId)
        return
      }
      if (fallbackTargetWindowId && fallbackTargetWindowId !== pending.sourceWindowId) {
        pending.dropped = true
        this.move({ tabId: pending.tabId, targetWindowId: fallbackTargetWindowId })
        return
      }

      if (!canDetach || !this.canDetachUnhandledDrag(pending)) {
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

  async closeMainWindowTabs() {
    if (this.mainCloseInFlight) {
      return false
    }

    this.mainCloseInFlight = true
    try {
      for (const tabId of [...this.mainTabIds]) {
        await this.options.closeTab(tabId)
        this.clearDragRecordsForTab(tabId)
        this.mainTabIds = this.mainTabIds.filter((entry) => entry !== tabId)
        this.emitPlacements()
      }
      return true
    } catch {
      return false
    } finally {
      this.mainCloseInFlight = false
    }
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
      return Boolean(
        !this.mainCloseInFlight && mainWindow && !mainWindow.isDestroyed() && mainWindow.webContents.id === sender.id
      )
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

  private resolveRecentHoveredDropTarget(drag: TabDragRecord, point: { x: number; y: number }) {
    if (
      !drag.hoverTargetWindowId ||
      !drag.hoverTargetAt ||
      Date.now() - drag.hoverTargetAt > DRAG_TARGET_HOVER_GRACE_MS
    ) {
      return null
    }

    const targetWindow = this.getWindow(drag.hoverTargetWindowId)
    if (!targetWindow || !this.isPointInsideVisibleWindow(point, targetWindow)) {
      return null
    }
    if (drag.hoverTargetWindowId === MAIN_WINDOW_ID) {
      return drag.hoverTargetWindowId
    }

    const targetRecord = this.detachedByWindowId.get(drag.hoverTargetWindowId)
    return targetRecord && this.canAcceptNativeDrop(targetRecord) ? drag.hoverTargetWindowId : null
  }

  private findWorkspaceDropTargetAtPoint(point: { x: number; y: number }, sourceWindowId: string) {
    const sourceWindow = this.getWindow(sourceWindowId)
    if (sourceWindow && this.isPointInsideVisibleWindow(point, sourceWindow)) {
      return sourceWindowId
    }

    const candidates: Array<{ windowId: string; window: BrowserWindow; priority: number }> = []
    const mainWindow = this.options.getMainWindow()
    if (sourceWindowId !== MAIN_WINDOW_ID && mainWindow) {
      candidates.push({ windowId: MAIN_WINDOW_ID, window: mainWindow, priority: 0 })
    }

    let detachedPriority = 1
    for (const [windowId, record] of this.detachedByWindowId) {
      if (windowId === sourceWindowId || !this.canAcceptNativeDrop(record)) {
        continue
      }
      candidates.push({ windowId, window: record.window, priority: detachedPriority++ })
    }

    return (
      candidates
        .filter(({ window }) => this.isPointInsideVisibleWindow(point, window))
        .sort((left, right) => {
          const focusDifference = Number(right.window.isFocused()) - Number(left.window.isFocused())
          return focusDifference || right.priority - left.priority
        })[0]?.windowId ?? null
    )
  }

  private canAcceptNativeDrop(record: DetachedWindowRecord) {
    return (
      !this.options.isQuitting() &&
      !record.window.isDestroyed() &&
      !record.window.webContents.isDestroyed() &&
      record.window.isVisible() &&
      record.ready &&
      !record.approvedClose &&
      !record.closeInFlight
    )
  }

  private isPointInsideVisibleWindow(point: { x: number; y: number }, window: BrowserWindow) {
    if (this.options.isQuitting() || window.isDestroyed() || window.webContents.isDestroyed() || !window.isVisible()) {
      return false
    }

    const bounds = window.getBounds()
    return (
      point.x >= bounds.x &&
      point.x < bounds.x + bounds.width &&
      point.y >= bounds.y &&
      point.y < bounds.y + bounds.height
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

    const existingMainWindow = this.options.getMainWindow()
    const mainWindow =
      existingMainWindow && !existingMainWindow.isDestroyed() ? existingMainWindow : this.options.ensureMainWindow()
    for (const tabId of [...record.tabIds]) {
      this.options.releaseTabRenderer(tabId, record.window.webContents)
      this.ownerWindowIdByTabId.delete(tabId)
      if (!mainWindow.isDestroyed()) {
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

  private closeEmptyMainWindow() {
    const mainWindow = this.options.getMainWindow()
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.destroy()
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
