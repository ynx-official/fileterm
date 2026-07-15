import { ipcMain } from 'electron'
import type {
  DetachWorkspaceTabInput,
  DropWorkspaceTabInput,
  FinishWorkspaceTabDragInput,
  MoveWorkspaceTabInput,
  WorkspaceTabDragInput
} from '@fileterm/core'
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
  ipcMain.handle('workspaceWindow:moveTab', (_event, input: MoveWorkspaceTabInput) => {
    workspaceWindowRegistry.move(input)
  })
  ipcMain.handle('workspaceWindow:startTabDrag', (event, input: WorkspaceTabDragInput) => {
    workspaceWindowRegistry.startDrag(input, event.sender)
  })
  ipcMain.handle('workspaceWindow:dropTab', (event, input: DropWorkspaceTabInput) => {
    workspaceWindowRegistry.drop(input, event.sender)
  })
  ipcMain.handle('workspaceWindow:finishTabDrag', (event, input: FinishWorkspaceTabDragInput) => {
    workspaceWindowRegistry.finishDrag(input, event.sender)
  })
  ipcMain.handle('workspaceWindow:claimTab', (event, tabId: string) => {
    workspaceWindowRegistry.claim(tabId, event.sender)
  })
}
