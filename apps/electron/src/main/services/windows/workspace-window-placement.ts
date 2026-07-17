import type { WorkspaceTabPlacement } from '@fileterm/core'

export type DetachedPlacement = {
  tabId: string
  ownerWindowId: string
  ready: boolean
  order: number
}

export function resolveWorkspaceTabPlacements(
  tabIds: readonly string[],
  detachedPlacements: readonly DetachedPlacement[],
  mainWindowId = 'main',
  mainTabOrder: readonly string[] = tabIds
): WorkspaceTabPlacement[] {
  const detachedByTabId = new Map(
    detachedPlacements.filter((placement) => placement.ready).map((placement) => [placement.tabId, placement])
  )
  const knownTabIds = new Set(tabIds)
  const orderedMainTabIds = [
    ...mainTabOrder.filter((tabId) => knownTabIds.has(tabId) && !detachedByTabId.has(tabId)),
    ...tabIds.filter((tabId) => !mainTabOrder.includes(tabId) && !detachedByTabId.has(tabId))
  ]
  const mainOrderByTabId = new Map(orderedMainTabIds.map((tabId, order) => [tabId, order]))

  return tabIds.map((tabId) => {
    const detached = detachedByTabId.get(tabId)
    if (detached) {
      return {
        tabId,
        ownerWindowId: detached.ownerWindowId,
        ownerKind: 'detached-session' as const,
        order: detached.order
      }
    }

    return {
      tabId,
      ownerWindowId: mainWindowId,
      ownerKind: 'main' as const,
      order: mainOrderByTabId.get(tabId) ?? orderedMainTabIds.length
    }
  })
}
