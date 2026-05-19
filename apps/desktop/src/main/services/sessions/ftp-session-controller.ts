import { randomUUID } from 'node:crypto'
import { readFile, stat, unlink, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { Client as BasicFtpClient } from 'basic-ftp'
import type { FtpProfile, FtpSessionController, RemoteFileItem } from '@termdock/core'
import { BaseFileSessionController } from './base-file-session-controller.js'
import { parentRemotePath, toFtpRemoteFileItem } from './session-file-utils.js'

export class LiveFtpSessionController extends BaseFileSessionController implements FtpSessionController {
  readonly type = 'ftp'

  private readonly ftp = new BasicFtpClient(20000)
  private currentRemotePath: string

  constructor(id: string, profile: FtpProfile) {
    super(id, 'ftp', profile)
    this.currentRemotePath = profile.remotePath || '/'
  }

  override async connect(): Promise<void> {
    const profile = this.profile as FtpProfile
    await this.ftp.access({
      host: profile.host,
      port: profile.port,
      user: profile.username,
      password: profile.password,
      secure: profile.secure
    })
    this.connected = true
    try {
      await this.ftp.cd(this.currentRemotePath)
      this.currentRemotePath = await this.ftp.pwd()
    } catch {
      this.currentRemotePath = profile.remotePath || '/'
    }
  }

  override async disconnect(): Promise<void> {
    this.ftp.close()
    this.connected = false
  }

  override getRemotePath(): string {
    return this.currentRemotePath
  }

  async abortTransfer(): Promise<void> {
    this.ftp.close()
    this.connected = false
  }

  async listRemoteFiles(): Promise<RemoteFileItem[]> {
    await this.ensureConnected()
    return this.readRemoteDirectory(this.currentRemotePath)
  }

  async openRemotePath(nextPath: string): Promise<RemoteFileItem[]> {
    await this.ensureConnected()
    await this.ftp.cd(nextPath)
    this.currentRemotePath = await this.ftp.pwd()
    return this.readRemoteDirectory(this.currentRemotePath)
  }

  async readRemoteFile(targetPath: string): Promise<string> {
    await this.ensureConnected()
    const localPath = this.tempFilePath(targetPath)
    try {
      await this.ftp.downloadTo(localPath, targetPath)
      return await readFile(localPath, 'utf8')
    } finally {
      void unlink(localPath).catch(() => undefined)
    }
  }

  async writeRemoteFile(targetPath: string, content: string): Promise<void> {
    await this.ensureConnected()
    const localPath = this.tempFilePath(targetPath)
    try {
      await writeFile(localPath, content, 'utf8')
      await this.ensureRemoteDirectory(path.posix.dirname(targetPath))
      await this.ftp.uploadFrom(localPath, targetPath)
    } finally {
      void unlink(localPath).catch(() => undefined)
    }
  }

  async ensureRemoteDirectory(targetPath: string): Promise<void> {
    await this.ensureConnected()
    if (!targetPath || targetPath === '.') {
      return
    }

    const previousPath = await this.ftp.pwd()
    try {
      await this.ftp.ensureDir(targetPath)
    } finally {
      await this.ftp.cd(previousPath)
    }
  }

  async uploadFile(localPath: string, remotePath: string, onProgress: (progress: number) => void): Promise<void> {
    await this.ensureConnected()
    const info = await stat(localPath)
    const total = Math.max(info.size, 1)
    this.ftp.trackProgress((progress) => {
      onProgress(Math.min(99, Math.round((progress.bytes / total) * 100)))
    })
    try {
      await this.ensureRemoteDirectory(path.posix.dirname(remotePath))
      await this.ftp.uploadFrom(localPath, remotePath)
      onProgress(100)
    } finally {
      this.ftp.trackProgress()
    }
  }

  async downloadFile(remotePath: string, localPath: string, onProgress: (progress: number) => void): Promise<void> {
    await this.ensureConnected()
    const total = Math.max(await this.ftp.size(remotePath), 1)
    this.ftp.trackProgress((progress) => {
      onProgress(Math.min(99, Math.round((progress.bytes / total) * 100)))
    })
    try {
      await this.ftp.downloadTo(localPath, remotePath)
      onProgress(100)
    } finally {
      this.ftp.trackProgress()
    }
  }

  private async readRemoteDirectory(targetPath: string): Promise<RemoteFileItem[]> {
    const entries = await this.ftp.list(targetPath)
    const rows = entries
      .filter((entry) => entry.name !== '.' && entry.name !== '..')
      .map((entry) => toFtpRemoteFileItem(targetPath, entry))
      .sort((left, right) => {
        if (left.type !== right.type) {
          return left.type === 'folder' ? -1 : 1
        }
        return left.name.localeCompare(right.name)
      })

    if (targetPath !== '/') {
      rows.unshift({
        path: parentRemotePath(targetPath),
        name: '..',
        type: 'folder',
        modified: '',
        size: '-',
        permission: '',
        ownerGroup: ''
      })
    }

    return rows
  }

  private tempFilePath(remotePath: string) {
    return path.join(os.tmpdir(), `termdock-${randomUUID()}-${path.posix.basename(remotePath) || 'remote-file'}`)
  }

  private async ensureConnected() {
    if (!this.connected) {
      await this.connect()
    }
  }
}
