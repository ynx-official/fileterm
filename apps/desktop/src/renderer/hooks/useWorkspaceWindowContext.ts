import { useEffect, useMemo, useState } from 'react'
import type { FileTermDesktopApi, WorkspaceTabPlacement, WorkspaceWindowContext } from '@fileterm/core'

const browserMainWindowContext: WorkspaceWindowContext = {
  windowId: 'main',
  kind: 'main'
}

export function useWorkspaceWindowContext({
  desktopApi,
  enabled
}: {
  desktopApi?: FileTermDesktopApi
  enabled: boolean
}) {
  const [context, setContext] = useState<WorkspaceWindowContext>(browserMainWindowContext)
  const [placements, setPlacements] = useState<WorkspaceTabPlacement[]>([])
  const [hasLoaded, setHasLoaded] = useState(!desktopApi || !enabled)

  useEffect(() => {
    if (!desktopApi || !enabled) {
      setContext(browserMainWindowContext)
      setPlacements([])
      setHasLoaded(true)
      return
    }

    let canceled = false
    setHasLoaded(false)
    const unsubscribe = desktopApi.onWorkspaceTabPlacementChanged((nextPlacements) => {
      if (!canceled) {
        setPlacements(nextPlacements)
      }
    })

    void Promise.all([desktopApi.getWorkspaceWindowContext(), desktopApi.getWorkspaceTabPlacements()])
      .then(([nextContext, nextPlacements]) => {
        if (canceled) {
          return
        }
        setContext(nextContext)
        setPlacements(nextPlacements)
        setHasLoaded(true)
      })
      .catch(() => {
        if (!canceled) {
          setHasLoaded(true)
        }
      })

    return () => {
      canceled = true
      unsubscribe()
    }
  }, [desktopApi, enabled])

  const placementsByTabId = useMemo(
    () => new Map(placements.map((placement) => [placement.tabId, placement])),
    [placements]
  )

  return {
    context,
    hasLoaded,
    placements,
    placementsByTabId
  }
}
