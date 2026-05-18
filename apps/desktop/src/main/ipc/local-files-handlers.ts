import { app, BrowserWindow, dialog, ipcMain } from 'electron'
import type { OpenDialogOptions } from 'electron'
import type { IpcServices } from './types.js'

export function registerLocalFilesHandlers(services: IpcServices) {
  const { localFilesService } = services

  ipcMain.handle('localFiles:listDirectory', (_, dirPath?: string) =>
    localFilesService.listDirectory(dirPath)
  )

  ipcMain.handle('localFiles:readFile', (_, filePath: string) =>
    localFilesService.readFile(filePath)
  )

  ipcMain.handle('localFiles:writeFile', (_, filePath: string, content: string) =>
    localFilesService.writeFile(filePath, content)
  )

  ipcMain.handle('localFiles:selectFiles', async (event, defaultPath?: string) => {
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options: OpenDialogOptions = {
      defaultPath,
      properties: ['openFile', 'openDirectory', 'multiSelections', 'createDirectory']
    }
    const result = window
      ? await dialog.showOpenDialog(window, options)
      : await dialog.showOpenDialog(options)
    return result.canceled ? [] : result.filePaths
  })

  ipcMain.handle('localFiles:selectDirectory', async (event, defaultPath?: string) => {
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options: OpenDialogOptions = {
      defaultPath: defaultPath || app.getPath('downloads'),
      properties: ['openDirectory', 'createDirectory']
    }
    const result = window
      ? await dialog.showOpenDialog(window, options)
      : await dialog.showOpenDialog(options)
    return result.canceled ? null : result.filePaths[0] ?? null
  })
}
