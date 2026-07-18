/**
 * xterm and Monaco cache glyph dimensions in their canvas renderers. In a
 * packaged webview a local font can finish loading, or the window can move to
 * a display with a different scale factor, after either component is mounted.
 * A regular DOM layout corrects itself in that situation; canvas-backed text
 * does not. Keep the font stack local to the app bundle and provide one
 * observer that asks consumers to remeasure at both boundaries.
 */
export const FILETERM_MONO_FONT_FAMILY = '"JetBrains Mono", "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'

const FILETERM_MONO_FONT_FAMILY_REFRESH =
  '"JetBrains Mono", "JetBrains Mono", "SF Mono", Menlo, Consolas, "Liberation Mono", monospace'

const MONO_FONT_LOADS = ['400 12px "JetBrains Mono"', '600 13px "JetBrains Mono"']

function nextPaint(callback: () => void) {
  const firstFrame = window.requestAnimationFrame(() => {
    window.requestAnimationFrame(callback)
  })
  return () => window.cancelAnimationFrame(firstFrame)
}

/**
 * Runs after bundled mono fonts become available and whenever WebView changes
 * device pixel ratio. The returned disposer prevents a closed standalone
 * editor from receiving a late font event.
 */
export function observeCanvasTextMetrics(onMetricsChanged: (fontFamily: string) => void) {
  let disposed = false
  let cancelPaint: (() => void) | null = null
  let useRefreshFontStack = false
  let pixelRatio = window.devicePixelRatio || 1
  let pixelRatioQuery = window.matchMedia(`(resolution: ${pixelRatio}dppx)`)

  const notify = () => {
    if (disposed) {
      return
    }
    cancelPaint?.()
    cancelPaint = nextPaint(() => {
      cancelPaint = null
      if (disposed) {
        return
      }
      useRefreshFontStack = !useRefreshFontStack
      onMetricsChanged(useRefreshFontStack ? FILETERM_MONO_FONT_FAMILY_REFRESH : FILETERM_MONO_FONT_FAMILY)
    })
  }

  const rebindPixelRatioQuery = () => {
    pixelRatioQuery.removeEventListener('change', onPixelRatioChanged)
    pixelRatio = window.devicePixelRatio || 1
    pixelRatioQuery = window.matchMedia(`(resolution: ${pixelRatio}dppx)`)
    pixelRatioQuery.addEventListener('change', onPixelRatioChanged)
  }

  const onPixelRatioChanged = () => {
    rebindPixelRatioQuery()
    notify()
  }

  const onViewportResize = () => {
    const nextPixelRatio = window.devicePixelRatio || 1
    if (Math.abs(nextPixelRatio - pixelRatio) > 0.001) {
      rebindPixelRatioQuery()
      notify()
    }
  }

  pixelRatioQuery.addEventListener('change', onPixelRatioChanged)
  window.addEventListener('resize', onViewportResize)
  window.visualViewport?.addEventListener('resize', onViewportResize)

  void Promise.all(MONO_FONT_LOADS.map((font) => document.fonts.load(font)))
    .catch(() => {
      // A system fallback remains usable if a local font cannot be decoded.
    })
    .finally(() => {
      void document.fonts.ready.then(notify).catch(notify)
    })

  return () => {
    disposed = true
    cancelPaint?.()
    pixelRatioQuery.removeEventListener('change', onPixelRatioChanged)
    window.removeEventListener('resize', onViewportResize)
    window.visualViewport?.removeEventListener('resize', onViewportResize)
  }
}
