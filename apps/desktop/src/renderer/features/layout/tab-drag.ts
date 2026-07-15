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
