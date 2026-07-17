import type { WorkspaceTabPlacement } from '@fileterm/core'

export function findTabMovedToWindow(
  previousPlacements: WorkspaceTabPlacement[],
  nextPlacements: WorkspaceTabPlacement[],
  targetWindowId: string
) {
  const previousByTabId = new Map(previousPlacements.map((placement) => [placement.tabId, placement]))

  return (
    nextPlacements.find((placement) => {
      const previous = previousByTabId.get(placement.tabId)
      return (
        previous !== undefined &&
        previous.ownerWindowId !== targetWindowId &&
        placement.ownerWindowId === targetWindowId
      )
    })?.tabId ?? null
  )
}
