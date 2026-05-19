import { ipcMain } from 'electron'
import type { IpcServices } from './types.js'

export function registerTransferHandlers(services: IpcServices) {
  const { workspaceService } = services

  ipcMain.handle('transfer:queueUpload', (_, fileNames: string[]) =>
    workspaceService.queueUpload(fileNames)
  )

  ipcMain.handle('transfer:cancel', (event, transferId: string) =>
    workspaceService.cancelTransfer(transferId, event.sender)
  )

  ipcMain.handle('transfer:uploadFile', (event, tabId: string, localPath: string, remoteDirectory: string) =>
    workspaceService.uploadFile(tabId, localPath, remoteDirectory, event.sender)
  )

  ipcMain.handle('transfer:downloadFile', (event, tabId: string, remotePath: string, localDirectory: string) =>
    workspaceService.downloadFile(tabId, remotePath, localDirectory, event.sender)
  )
}
