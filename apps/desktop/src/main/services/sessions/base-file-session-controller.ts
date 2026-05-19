import type { ConnectionProfile, FileSessionController, RemoteFileItem } from '@termdock/core'

export abstract class BaseFileSessionController implements FileSessionController {
  readonly id: string
  readonly type: 'ssh' | 'ftp'

  protected connected = false

  constructor(
    id: string,
    type: 'ssh' | 'ftp',
    protected readonly profile: ConnectionProfile
  ) {
    this.id = id
    this.type = type
  }

  async connect(): Promise<void> {
    this.connected = true
  }

  async disconnect(): Promise<void> {
    this.connected = false
  }

  getSummary(): string {
    return this.connected
      ? `Connected to ${this.profile.host}:${this.profile.port}`
      : `Ready to connect ${this.profile.host}:${this.profile.port}`
  }

  getRemotePath(): string {
    return this.profile.remotePath
  }

  abstract listRemoteFiles(): Promise<RemoteFileItem[]>
  abstract openRemotePath(path: string): Promise<RemoteFileItem[]>
  abstract readRemoteFile(path: string): Promise<string>
  abstract writeRemoteFile(path: string, content: string): Promise<void>
  abstract ensureRemoteDirectory(path: string): Promise<void>
  abstract abortTransfer(): Promise<void>
  abstract uploadFile(localPath: string, remotePath: string, onProgress: (progress: number) => void): Promise<void>
  abstract downloadFile(remotePath: string, localPath: string, onProgress: (progress: number) => void): Promise<void>
}
