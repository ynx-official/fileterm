import { useEffect } from 'react'

export type ThemeMode = 'default-dark' | 'default-light'

export function useThemeMode(themeName: ThemeMode = 'default-dark') {
  useEffect(() => {
    document.documentElement.dataset.theme = themeName
    return () => {
      if (document.documentElement.dataset.theme === themeName) {
        delete document.documentElement.dataset.theme
      }
    }
  }, [themeName])
}
