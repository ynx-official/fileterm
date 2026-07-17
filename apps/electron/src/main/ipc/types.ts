import { BrowserWindow } from 'electron'
import type { WebContents } from 'electron'
import type { LocalFilesService } from '../services/local-files-service.js'
import type { SshKeyService } from '../services/ssh-keys/ssh-key-service.js'
import type { WorkspaceService } from '../services/workspace-service.js'
import type { AppUpdateService } from '../services/app-update-service.js'
import type { WorkspaceWindowRegistry } from '../services/windows/workspace-window-registry.js'
import type { DetachWorkspaceTabInput, WorkspaceWindowContext } from '@fileterm/core'

export interface IpcWindowOptions {
  getMainWindow(): BrowserWindow | null
  ensureMainWindow(): BrowserWindow
  getUiPreferences(): { theme: 'default-dark' | 'default-light'; locale: 'zhCN' | 'enUS' }
  setUiPreferences(input: Partial<{ theme: 'default-dark' | 'default-light'; locale: 'zhCN' | 'enUS' }>): {
    theme: 'default-dark' | 'default-light'
    locale: 'zhCN' | 'enUS'
  }
  getUiStateItem(key: string): Promise<string | null>
  setUiStateItem(key: string, value: string): Promise<void>
  removeUiStateItem(key: string): Promise<void>
  createDetachedWorkspaceWindow(context: WorkspaceWindowContext, input: DetachWorkspaceTabInput): BrowserWindow
  isQuitting(): boolean
  openConnectionManagerWindow(parent: BrowserWindow): void
  openCommandManagerWindow(parent: BrowserWindow): void
  openConnectionFormWindow(parent: BrowserWindow, mode: 'create' | 'edit', profileId?: string): void
  openCommandFormWindow(parent: BrowserWindow, mode: 'create' | 'edit', commandId?: string, folderId?: string): void
  openFileEditorWindow(
    parent: BrowserWindow,
    input: { source: 'local' | 'remote'; path: string; name: string; tabId?: string; encoding?: string }
  ): void
  confirmCloseFileEditorWindow(window: BrowserWindow): void
  cancelCloseFileEditorWindow(window: BrowserWindow): void
  openLogsDirectory(): Promise<void>
  appUpdateService: AppUpdateService
  requestQuitApp(): void
  confirmCloseWindow(action: 'quit' | 'hide' | 'close-workspace' | 'cancel'): void | Promise<void>
}

export interface IpcServices {
  workspaceService: WorkspaceService
  workspaceWindowRegistry: WorkspaceWindowRegistry
  sshKeyService: SshKeyService
  localFilesService: LocalFilesService
  broadcastSnapshot(snapshot: Awaited<ReturnType<WorkspaceService['getSnapshot']>>): void
}

export function resolveWindow(sender: WebContents, options: IpcWindowOptions) {
  return BrowserWindow.fromWebContents(sender) ?? options.getMainWindow() ?? undefined
}
