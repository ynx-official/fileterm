import { useEffect, useRef, type PointerEvent as ReactPointerEvent } from 'react'

export type PointerSortTarget = {
  id: string
  kind: string
  element: HTMLElement
}

type PointerSortState<T> = {
  source: T
  sourceElement: HTMLElement
  pressX: number
  pressY: number
  originLeft: number
  originTop: number
  dragStartX: number
  dragStartY: number
  pointerId: number
  active: boolean
  /**
   * 拖出分离标志：当 onDetach 提供时，pointer 离开窗口可视区域后置 true，
   * 后续 onTarget 不再触发；pointerup 时若仍为 true 则走 onDetach 而非 onDrop。
   * 重新回到窗口内会复位为 false，允许用户拖出后再拖回取消分离。
   */
  detaching: boolean
} | null

const TARGET_SELECTOR = '[data-fileterm-sort-id]'
const INTERACTIVE_SELECTOR = 'button, input, textarea, select, a, [contenteditable="true"]'
const DETACHING_BODY_CLASS = 'fileterm-tab-detaching'

/**
 * Tauri's macOS WebView does not consistently complete HTML5 drag/drop for
 * in-page sortable rows. This uses pointer movement as the authoritative
 * local-sort path while leaving native drag/drop available for external files.
 *
 * 可选 `onDetach` 用于实现"拖出窗口分离"信号：拖动过程中 pointer 离开
 * 当前窗口可视区域时，转入 detaching 状态；在 detaching 状态下释放
 * pointer，调用 `onDetach(source, clientX, clientY)` 而不是 `onDrop`。
 * 调用方负责把 clientX/Y 转换为 Tauri 物理屏幕坐标并触发 Rust 侧
 * finishWorkspaceTabDrag。
 */
export function usePointerSortFallback<T>({
  onStart,
  onTarget,
  onDrop,
  onCancel,
  onDetach
}: {
  onStart(source: T): void
  onTarget(source: T, target: PointerSortTarget, clientY: number): void
  onDrop(source: T, target: PointerSortTarget | null, clientY: number): void
  onCancel(): void
  onDetach?(source: T, clientX: number, clientY: number): void
}) {
  const stateRef = useRef<PointerSortState<T>>(null)
  const ghostRef = useRef<HTMLElement | null>(null)
  const callbacksRef = useRef({ onStart, onTarget, onDrop, onCancel, onDetach })
  callbacksRef.current = { onStart, onTarget, onDrop, onCancel, onDetach }

  useEffect(() => {
    const resolveTarget = (clientX: number, clientY: number): PointerSortTarget | null => {
      const element = document.elementFromPoint(clientX, clientY)?.closest<HTMLElement>(TARGET_SELECTOR)
      if (!element) return null
      const id = element.dataset.filetermSortId
      const kind = element.dataset.filetermSortKind
      return id && kind ? { id, kind, element } : null
    }

    const removeGhost = () => {
      ghostRef.current?.remove()
      ghostRef.current = null
      document.documentElement.classList.remove('fileterm-pointer-sorting')
      document.documentElement.classList.remove(DETACHING_BODY_CLASS)
    }

    const moveGhost = (state: NonNullable<PointerSortState<T>>, clientX: number, clientY: number) => {
      const ghost = ghostRef.current
      if (!ghost) return
      ghost.style.transform = `translate3d(${Math.round(state.originLeft + clientX - state.dragStartX)}px, ${Math.round(state.originTop + clientY - state.dragStartY)}px, 0)`
    }

    const createGhost = (sourceElement: HTMLElement, clientX: number, clientY: number) => {
      removeGhost()
      const rect = sourceElement.getBoundingClientRect()
      const computed = window.getComputedStyle(sourceElement)
      const ghost = sourceElement.cloneNode(true) as HTMLElement
      ghost.classList.add('fileterm-pointer-sort-ghost')
      ghost.setAttribute('aria-hidden', 'true')
      Object.assign(ghost.style, {
        position: 'fixed',
        inset: '0 auto auto 0',
        display: computed.display,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
        gridTemplateColumns: computed.gridTemplateColumns,
        columnGap: computed.columnGap,
        padding: computed.padding,
        border: computed.border,
        borderRadius: computed.borderRadius,
        background: computed.background,
        color: computed.color,
        font: computed.font,
        boxSizing: computed.boxSizing,
        opacity: '0.78',
        pointerEvents: 'none',
        zIndex: '2147483647',
        overflow: 'hidden',
        boxShadow: '0 3px 10px rgb(0 0 0 / 18%)',
        transition: 'none',
        willChange: 'transform'
      })
      document.body.appendChild(ghost)
      ghostRef.current = ghost
      const state = stateRef.current
      if (state) moveGhost(state, clientX, clientY)
    }

    const handlePointerMove = (event: PointerEvent) => {
      const state = stateRef.current
      if (!state) return

      if (!state.active) {
        const dx = event.clientX - state.pressX
        const dy = event.clientY - state.pressY
        if (dx * dx + dy * dy < 25) return
        state.active = true
        // Match Chromium's native drag image: when dragging actually starts,
        // its left/top still match the source row; later movement translates
        // the image by the cursor's delta from this exact start frame.
        state.dragStartX = event.clientX
        state.dragStartY = event.clientY
        createGhost(state.sourceElement, event.clientX, event.clientY)
        document.documentElement.classList.add('fileterm-pointer-sorting')
        callbacksRef.current.onStart(state.source)
      }

      moveGhost(state, event.clientX, event.clientY)
      // 拖出分离检测：仅当 onDetach 被提供时启用。pointer 离开当前窗口
      // 可视区域时进入 detaching 状态，回到窗口内则退出。detaching 状态
      // 下不触发 onTarget，避免在窗口外的 ghost 误命中不可见的目标。
      if (callbacksRef.current.onDetach) {
        const outsideWindow =
          event.clientX < 0 ||
          event.clientX > window.innerWidth ||
          event.clientY < 0 ||
          event.clientY > window.innerHeight
        if (outsideWindow !== state.detaching) {
          state.detaching = outsideWindow
          if (outsideWindow) {
            document.documentElement.classList.add(DETACHING_BODY_CLASS)
          } else {
            document.documentElement.classList.remove(DETACHING_BODY_CLASS)
          }
        }
      }
      if (!state.detaching) {
        const target = resolveTarget(event.clientX, event.clientY)
        if (target) callbacksRef.current.onTarget(state.source, target, event.clientY)
      }
    }

    const handlePointerUp = (event: PointerEvent) => {
      const state = stateRef.current
      if (!state) return
      stateRef.current = null
      if (state.sourceElement.hasPointerCapture(state.pointerId)) {
        state.sourceElement.releasePointerCapture(state.pointerId)
      }
      removeGhost()
      if (!state.active) return
      // 拖出分离：用最后已知的 client 坐标回调，调用方负责转换为屏幕坐标。
      if (state.detaching && callbacksRef.current.onDetach) {
        callbacksRef.current.onDetach(state.source, event.clientX, event.clientY)
        return
      }
      callbacksRef.current.onDrop(state.source, resolveTarget(event.clientX, event.clientY), event.clientY)
    }

    const handlePointerCancel = () => {
      const state = stateRef.current
      if (!state) return
      stateRef.current = null
      if (state.sourceElement.hasPointerCapture(state.pointerId)) {
        state.sourceElement.releasePointerCapture(state.pointerId)
      }
      removeGhost()
      callbacksRef.current.onCancel()
    }

    window.addEventListener('pointermove', handlePointerMove, true)
    window.addEventListener('pointerup', handlePointerUp, true)
    window.addEventListener('pointercancel', handlePointerCancel, true)
    window.addEventListener('lostpointercapture', handlePointerCancel, true)
    window.addEventListener('blur', handlePointerCancel)
    return () => {
      window.removeEventListener('pointermove', handlePointerMove, true)
      window.removeEventListener('pointerup', handlePointerUp, true)
      window.removeEventListener('pointercancel', handlePointerCancel, true)
      window.removeEventListener('lostpointercapture', handlePointerCancel, true)
      window.removeEventListener('blur', handlePointerCancel)
      removeGhost()
    }
  }, [])

  return (event: ReactPointerEvent, source: T) => {
    if (event.button !== 0 || !event.isPrimary) return
    if (event.target instanceof Element && event.target.closest(INTERACTIVE_SELECTOR)) return
    const rect = event.currentTarget.getBoundingClientRect()
    try {
      event.currentTarget.setPointerCapture(event.pointerId)
    } catch {
      // Window-level listeners remain the fallback on WebViews that reject
      // pointer capture for a synthetic or already-released pointer.
    }
    stateRef.current = {
      source,
      sourceElement: event.currentTarget as HTMLElement,
      pressX: event.clientX,
      pressY: event.clientY,
      originLeft: rect.left,
      originTop: rect.top,
      dragStartX: event.clientX,
      dragStartY: event.clientY,
      pointerId: event.pointerId,
      active: false,
      detaching: false
    }
  }
}
