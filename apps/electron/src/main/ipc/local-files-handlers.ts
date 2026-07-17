import { app, BrowserWindow, dialog, ipcMain } from 'electron'
import type { OpenDialogOptions } from 'electron'
import type { PermissionChangeOptions } from '@fileterm/core'
import { appError, appLog } from '../services/app-logger.js'
import type { IpcServices } from './types.js'

export function registerLocalFilesHandlers(services: IpcServices) {
  const { localFilesService } = services

  ipcMain.handle('localFiles:listDirectory', (_, dirPath?: string) => localFilesService.listDirectory(dirPath))

  ipcMain.handle('localFiles:readFile', (_, filePath: string, encoding?: string) =>
    localFilesService.readFile(filePath, encoding)
  )

  ipcMain.handle('localFiles:writeFile', (_, filePath: string, content: string, encoding?: string) =>
    localFilesService.writeFile(filePath, content, encoding)
  )

  ipcMain.handle('localFiles:createDirectory', (_, dirPath: string, name: string) =>
    localFilesService.createDirectory(dirPath, name)
  )

  ipcMain.handle('localFiles:createFile', (_, dirPath: string, name: string) =>
    localFilesService.createFile(dirPath, name)
  )

  ipcMain.handle('localFiles:copyPath', (_, sourcePath: string, destinationPath: string) =>
    localFilesService.copyPath(sourcePath, destinationPath)
  )

  ipcMain.handle('localFiles:movePath', (_, sourcePath: string, destinationPath: string) =>
    localFilesService.movePath(sourcePath, destinationPath)
  )

  ipcMain.handle('localFiles:renamePath', (_, targetPath: string, newName: string) =>
    localFilesService.renamePath(targetPath, newName)
  )

  ipcMain.handle('localFiles:deletePath', (_, targetPath: string) => localFilesService.deletePath(targetPath))

  ipcMain.handle('localFiles:changePermissions', (_, targetPath: string, options: PermissionChangeOptions) =>
    localFilesService.changePermissions(targetPath, options)
  )

  ipcMain.handle('localFiles:selectFiles', async (event, defaultPath?: string) => {
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options: OpenDialogOptions = {
      defaultPath,
      properties: ['openFile', 'openDirectory', 'multiSelections', 'createDirectory']
    }
    try {
      const result = window ? await dialog.showOpenDialog(window, options) : await dialog.showOpenDialog(options)
      appLog('[FileTerm][Local] Select files completed', {
        defaultPath,
        canceled: result.canceled,
        count: result.filePaths.length
      })
      return result.canceled ? [] : result.filePaths
    } catch (error) {
      appError('[FileTerm][Local] Select files failed', { defaultPath, error: describeLocalError(error) })
      throw error
    }
  })

  ipcMain.handle('localFiles:selectDirectory', async (event, defaultPath?: string) => {
    const window = BrowserWindow.fromWebContents(event.sender) ?? undefined
    const options: OpenDialogOptions = {
      defaultPath: defaultPath || app.getPath('downloads'),
      properties: ['openDirectory', 'createDirectory']
    }
    try {
      const result = window ? await dialog.showOpenDialog(window, options) : await dialog.showOpenDialog(options)
      appLog('[FileTerm][Local] Select directory completed', {
        defaultPath: options.defaultPath,
        canceled: result.canceled,
        selectedPath: result.filePaths[0] ?? null
      })
      return result.canceled ? null : (result.filePaths[0] ?? null)
    } catch (error) {
      appError('[FileTerm][Local] Select directory failed', {
        defaultPath: options.defaultPath,
        error: describeLocalError(error)
      })
      throw error
    }
  })
}

function describeLocalError(error: unknown) {
  if (!(error instanceof Error)) {
    return error
  }

  const filesystemError = error as NodeJS.ErrnoException
  return {
    name: error.name,
    message: error.message,
    code: filesystemError.code,
    errno: filesystemError.errno,
    syscall: filesystemError.syscall,
    path: filesystemError.path,
    stack: error.stack
  }
}
