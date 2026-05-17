import { useEffect } from 'react'

export function useThemeMode(themeName = 'default') {
  useEffect(() => {
    document.documentElement.dataset.theme = themeName
    return () => {
      if (document.documentElement.dataset.theme === themeName) {
        delete document.documentElement.dataset.theme
      }
    }
  }, [themeName])
}
