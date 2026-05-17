import { ipcMain } from 'electron'
import type { IpcServices } from './types.js'

export function registerRemoteFilesHandlers(services: IpcServices) {
  const { workspaceService } = services

  ipcMain.handle('remoteFiles:openPath', (_, tabId: string, targetPath: string) =>
    workspaceService.openRemotePath(tabId, targetPath)
  )

  ipcMain.handle('remoteFiles:readFile', (_, tabId: string, targetPath: string) =>
    workspaceService.readRemoteFile(tabId, targetPath)
  )

  ipcMain.handle('remoteFiles:writeFile', (_, tabId: string, targetPath: string, content: string) =>
    workspaceService.writeRemoteFile(tabId, targetPath, content)
  )
}
