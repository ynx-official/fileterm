import { app, BrowserWindow } from 'electron'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { registerIpcHandlers } from './ipc/index.js'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

let mainWindow: BrowserWindow | null = null
let connectionManagerWindow: BrowserWindow | null = null
let connectionFormWindow: BrowserWindow | null = null

const isMac = process.platform === 'darwin'

function loadAppWindow(win: BrowserWindow, searchParams?: Record<string, string>) {
  if (!app.isPackaged) {
    const url = new URL('http://localhost:5188')
    Object.entries(searchParams ?? {}).forEach(([key, value]) => {
      url.searchParams.set(key, value)
    })
    win.loadURL(url.toString())
    return
  }

  win.loadFile(path.join(__dirname, '../../dist/index.html'), {
    query: searchParams
  })
}

function createMainWindow() {
  const win = new BrowserWindow({
    width: 1360,
    height: 900,
    minWidth: 1040,
    minHeight: 680,
    title: 'TermDock',
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 20, y: 18 },
    backgroundColor: '#151515',
    webPreferences: {
      preload: path.join(__dirname, '../preload/preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  })

  mainWindow = win
  win.on('closed', () => {
    if (mainWindow === win) {
      mainWindow = null
    }
  })

  if (!app.isPackaged) {
    loadAppWindow(win)
    win.webContents.on('did-finish-load', async () => {
      try {
        const hasDesktopApi = await win.webContents.executeJavaScript('Boolean(window.termdock?.isDesktop)')
        console.log(`[TermDock] preload ready: ${hasDesktopApi}`)
      } catch (error) {
        console.error('[TermDock] preload probe failed', error)
      }
    })
    win.webContents.openDevTools({ mode: 'detach' })
    return win
  }

  loadAppWindow(win)
  return win
}

function openConnectionManagerWindow(parent: BrowserWindow) {
  void parent
  if (connectionManagerWindow && !connectionManagerWindow.isDestroyed()) {
    connectionManagerWindow.focus()
    return
  }

  const win = new BrowserWindow({
    width: 980,
    height: 680,
    minWidth: 760,
    minHeight: 520,
    show: false,
    title: '连接管理器',
    backgroundColor: '#1b1b1b',
    autoHideMenuBar: true,
    titleBarStyle: isMac ? 'hiddenInset' : 'default',
    trafficLightPosition: isMac ? { x: 16, y: 14 } : undefined,
    minimizable: false,
    webPreferences: {
      preload: path.join(__dirname, '../preload/preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  })

  connectionManagerWindow = win
  win.once('ready-to-show', () => {
    win.show()
  })
  win.on('closed', () => {
    if (connectionManagerWindow === win) {
      connectionManagerWindow = null
    }
  })

  loadAppWindow(win, { window: 'connection-manager' })
}

function openConnectionFormWindow(parent: BrowserWindow, mode: 'create' | 'edit', profileId?: string) {
  void parent
  if (connectionFormWindow && !connectionFormWindow.isDestroyed()) {
    connectionFormWindow.close()
  }

  const win = new BrowserWindow({
    width: 920,
    height: 720,
    minWidth: 760,
    minHeight: 620,
    show: false,
    title: mode === 'edit' ? '编辑连接' : '新建连接',
    backgroundColor: '#1b1b1b',
    autoHideMenuBar: true,
    titleBarStyle: isMac ? 'hiddenInset' : 'default',
    trafficLightPosition: isMac ? { x: 16, y: 14 } : undefined,
    minimizable: false,
    webPreferences: {
      preload: path.join(__dirname, '../preload/preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false
    }
  })

  connectionFormWindow = win
  win.once('ready-to-show', () => {
    win.show()
  })
  win.on('closed', () => {
    if (connectionFormWindow === win) {
      connectionFormWindow = null
    }
  })

  loadAppWindow(win, {
    window: 'connection-form',
    mode,
    ...(profileId ? { profileId } : {})
  })
}

app.whenReady().then(() => {
  registerIpcHandlers(app.getPath('userData'), {
    getMainWindow: () => mainWindow,
    openConnectionManagerWindow,
    openConnectionFormWindow
  })
  createMainWindow()

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createMainWindow()
    }
  })
})

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit()
  }
})
