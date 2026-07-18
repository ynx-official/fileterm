import { useEffect, useMemo, useState } from 'react'
import type { FileTermDesktopApi, WorkspaceTabPlacement, WorkspaceWindowContext } from '@fileterm/core'

// 没有桌面 API（浏览器预览）或未启用（连接管理器、文件编辑器等子窗口）
// 时，回退为主窗口身份。这类窗口不渲染工作区标签，placement 列表为空。
const browserMainWindowContext: WorkspaceWindowContext = {
  windowId: 'main',
  kind: 'main'
}

export type UseWorkspaceWindowContextOptions = {
  desktopApi?: FileTermDesktopApi
  enabled: boolean
}

export type UseWorkspaceWindowContextResult = {
  context: WorkspaceWindowContext
  hasLoaded: boolean
  placements: WorkspaceTabPlacement[]
  placementsByTabId: Map<string, WorkspaceTabPlacement>
}

/**
 * 订阅当前窗口的身份与所有标签归属（WorkspaceTabPlacement）。
 *
 * - 主窗口：context.windowId === 'main'，可见标签 = placements 中 ownerWindowId
 *   为 'main' 的标签 + 没有任何 placement 记录的标签（fail-safe）。
 * - 独立会话窗口：context.windowId 是该窗口的稳定 ID，可见标签 = placements
 *   中 ownerWindowId 等于该 ID 的标签。
 *
 * 实际的归属过滤在 useWorkspaceTabs 中执行，这里只提供原始数据。
 *
 * Hydration 模式与 useWorkspaceIpcSync 的 onWorkspaceSnapshot 一致：
 * 先订阅事件，再发起 IPC 取初值，避免错过订阅窗口期内广播的事件；
 * 若事件先到达，则 IPC 结果不再覆盖（防止回退到旧值）。
 */
export function useWorkspaceWindowContext({
  desktopApi,
  enabled
}: UseWorkspaceWindowContextOptions): UseWorkspaceWindowContextResult {
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
    let receivedPlacementEvent = false
    setHasLoaded(false)

    const unsubscribe = desktopApi.onWorkspaceTabPlacementsChanged((nextPlacements) => {
      if (canceled) {
        return
      }
      receivedPlacementEvent = true
      setPlacements(nextPlacements)
    })

    void Promise.all([desktopApi.getWorkspaceWindowContext(), desktopApi.getWorkspaceTabPlacements()])
      .then(([nextContext, nextPlacements]) => {
        if (canceled) {
          return
        }
        setContext(nextContext)
        // 事件已经先到达时不要用 IPC 初值覆盖，避免把更新的归属信息回退。
        if (!receivedPlacementEvent) {
          setPlacements(nextPlacements)
        }
        setHasLoaded(true)
      })
      .catch(() => {
        // 即使初始化失败也允许渲染：上层会看到空 placements，主窗口回退
        // 到"所有标签都属于 main"的默认行为，独立窗口暂时看不到标签。
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
