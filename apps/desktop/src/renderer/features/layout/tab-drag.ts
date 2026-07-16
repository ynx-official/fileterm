export const WORKSPACE_TAB_DRAG_MIME = 'application/x-fileterm-workspace-tab'
export const FILETERM_TAB_DRAG_MIME = 'application/x-fileterm-tab'
export const WORKSPACE_TAB_PRECISE_DROP_SELECTOR = '[data-workspace-tab-drop-zone="precise"]'

export type TabDragEndState = {
  screenX: number
  screenY: number
}

export type WindowScreenBounds = {
  x: number
  y: number
  width: number
  height: number
}

export type DragDataTransferLike = {
  types: ArrayLike<string>
}

export function isWorkspaceTabDrag(dataTransfer: DragDataTransferLike | null | undefined) {
  if (!dataTransfer) {
    return false
  }

  const types = Array.from(dataTransfer.types)
  return types.includes(WORKSPACE_TAB_DRAG_MIME) || types.includes(FILETERM_TAB_DRAG_MIME)
}

export function isWorkspaceTabPreciseDropTarget(target: EventTarget | null) {
  return (
    typeof Element !== 'undefined' &&
    target instanceof Element &&
    target.closest(WORKSPACE_TAB_PRECISE_DROP_SELECTOR) !== null
  )
}

export type WorkspaceTabDropIndexInput = {
  sessionTabIds: string[]
  draggedTabId: string
  isSameWindow: boolean
  targetTabId?: string | null
  preserveCurrentOrder?: boolean
}

export function resolveWorkspaceTabDropTargetIndex({
  sessionTabIds,
  draggedTabId,
  isSameWindow,
  targetTabId,
  preserveCurrentOrder = false
}: WorkspaceTabDropIndexInput) {
  const sourceIndex = isSameWindow ? sessionTabIds.indexOf(draggedTabId) : -1
  let targetIndex =
    preserveCurrentOrder && sourceIndex >= 0
      ? sourceIndex
      : targetTabId
        ? sessionTabIds.indexOf(targetTabId)
        : sessionTabIds.length
  if (!preserveCurrentOrder && sourceIndex >= 0 && targetIndex > sourceIndex) {
    targetIndex -= 1
  }
  return Math.max(0, targetIndex)
}

export function canDetachWorkspaceTabFromWindow(windowKind: 'main' | 'detached-session', sessionTabCount: number) {
  return windowKind === 'main' || sessionTabCount > 1
}

export function resolveWorkspaceTabOutsideFeedback(isSessionTab: boolean, canDetach: boolean) {
  if (!isSessionTab) {
    return 'blocked' as const
  }
  return canDetach ? ('detach' as const) : ('attach' as const)
}

export function isTabDragReleasedOutsideWindow(dragEnd: TabDragEndState, windowBounds: WindowScreenBounds, margin = 8) {
  // Chromium may clear dragend coordinates when the pointer is released outside
  // the native window. The document dragleave state handles that ambiguous case.
  if (dragEnd.screenX === 0 && dragEnd.screenY === 0) {
    return false
  }

  const right = windowBounds.x + windowBounds.width
  const bottom = windowBounds.y + windowBounds.height
  return (
    dragEnd.screenX < windowBounds.x - margin ||
    dragEnd.screenX > right + margin ||
    dragEnd.screenY < windowBounds.y - margin ||
    dragEnd.screenY > bottom + margin
  )
}
