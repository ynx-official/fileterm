import type { WorkspaceTabPlacement } from '@fileterm/core'

export type DetachedPlacement = {
  tabId: string
  ownerWindowId: string
  ready: boolean
}

export function resolveWorkspaceTabPlacements(
  tabIds: readonly string[],
  detachedPlacements: readonly DetachedPlacement[],
  mainWindowId = 'main'
): WorkspaceTabPlacement[] {
  const detachedByTabId = new Map(
    detachedPlacements.filter((placement) => placement.ready).map((placement) => [placement.tabId, placement])
  )

  return tabIds.map((tabId) => {
    const detached = detachedByTabId.get(tabId)
    return detached
      ? {
          tabId,
          ownerWindowId: detached.ownerWindowId,
          ownerKind: 'detached-session' as const
        }
      : {
          tabId,
          ownerWindowId: mainWindowId,
          ownerKind: 'main' as const
        }
  })
}
