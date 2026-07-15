import { ipcMain } from 'electron'
import type { DetachWorkspaceTabInput } from '@fileterm/core'
import type { IpcServices } from './types.js'

export function registerWorkspaceWindowHandlers(services: IpcServices) {
  const { workspaceWindowRegistry } = services

  ipcMain.handle('workspaceWindow:getContext', (event) => workspaceWindowRegistry.getContext(event.sender))
  ipcMain.handle('workspaceWindow:getPlacements', () => workspaceWindowRegistry.listPlacements())
  ipcMain.handle('workspaceWindow:detachTab', (_event, input: DetachWorkspaceTabInput) => {
    workspaceWindowRegistry.detach(input)
  })
  ipcMain.handle('workspaceWindow:attachTab', (_event, tabId: string) => {
    workspaceWindowRegistry.attach(tabId)
  })
  ipcMain.handle('workspaceWindow:claimTab', (event, tabId: string) => {
    workspaceWindowRegistry.claim(tabId, event.sender)
  })
}
