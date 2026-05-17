import { readFile, stat } from 'node:fs/promises'
import type { ClientChannel, FileEntry, SFTPWrapper } from 'ssh2'
import { Client } from 'ssh2'
import type { FileSessionController, RemoteFileItem, SystemMetrics, SshProfile, SshSessionController } from '@termdock/core'
import { BaseFileSessionController } from './base-file-session-controller.js'
import { buildMetricsCommand, parentRemotePath, parseSystemMetrics, toRemoteFileItem } from './session-file-utils.js'

export class LiveSshSessionController extends BaseFileSessionController implements SshSessionController {
  readonly type = 'ssh'

  private readonly ssh = new Client()
  private sftp?: SFTPWrapper
  private shellStream?: {
    write(data: string): void
    setWindow(rows: number, cols: number, height: number, width: number): void
    end(): void
  }
  private transcript = ''
  private currentRemotePath: string
  private metrics?: SystemMetrics

  constructor(
    id: string,
    profile: SshProfile,
    private readonly onData: (chunk: string) => void,
    private readonly onStateChange: (summary: string, transcript: string, connected: boolean) => void
  ) {
    super(id, 'ssh', profile)
    this.currentRemotePath = profile.remotePath || '.'
    this.appendSystemMessage('连接主机...\r\n')
  }

  override async connect(): Promise<void> {
    const profile = this.profile as SshProfile
    const privateKey = profile.authType === 'privateKey' && profile.privateKeyPath
      ? await readFile(profile.privateKeyPath, 'utf8')
      : undefined

    await new Promise<void>((resolve, reject) => {
      let settled = false

      this.ssh
        .on('ready', () => {
          this.connected = true
          this.appendSystemMessage('连接主机成功\r\n')
          this.onStateChange(this.getSummary(), this.transcript, true)
          this.ssh.shell(
            {
              term: 'xterm-256color',
              rows: 32,
              cols: 120
            },
            (error: Error | undefined, stream: ClientChannel) => {
              if (error) {
                if (!settled) {
                  settled = true
                  reject(error)
                }
                return
              }

              this.shellStream = stream
              stream.on('data', (chunk: Buffer) => {
                const text = chunk.toString('utf8')
                this.transcript += text
                this.onData(text)
                this.onStateChange(this.getSummary(), this.transcript, true)
              })
              stream.on('close', () => {
                this.connected = false
                this.onStateChange('Shell closed', this.transcript, false)
              })

              if (!settled) {
                settled = true
                resolve()
              }
            }
          )
        })
        .on('error', (error: Error) => {
          this.connected = false
          this.appendSystemMessage(`连接失败: ${error.message}\r\n`)
          if (!settled) {
            settled = true
            reject(error)
          }
          this.onStateChange(`Connection error: ${error.message}`, this.transcript, false)
        })
        .on('close', () => {
          this.connected = false
          this.appendSystemMessage('连接已断开\r\n')
          this.onStateChange('Disconnected', this.transcript, false)
        })
        .connect({
          host: profile.host,
          port: profile.port,
          username: profile.username,
          password: profile.authType === 'password' ? profile.password : undefined,
          privateKey,
          passphrase: profile.passphrase,
          readyTimeout: 15000,
          tryKeyboard: profile.authType === 'password'
        })
    })
  }

  override async disconnect(): Promise<void> {
    this.shellStream?.end()
    this.ssh.end()
    this.connected = false
  }

  getTerminalTranscript(): string {
    return this.transcript
  }

  override getRemotePath(): string {
    return this.currentRemotePath
  }

  getSystemMetrics(): SystemMetrics | undefined {
    return this.metrics
  }

  async write(data: string): Promise<void> {
    this.shellStream?.write(data)
  }

  async resize(cols: number, rows: number): Promise<void> {
    this.shellStream?.setWindow(rows, cols, rows * 16, cols * 8)
  }

  async listRemoteFiles(): Promise<RemoteFileItem[]> {
    return this.readRemoteDirectory(this.currentRemotePath)
  }

  async openRemotePath(nextPath: string): Promise<RemoteFileItem[]> {
    this.currentRemotePath = nextPath
    return this.readRemoteDirectory(this.currentRemotePath)
  }

  async readRemoteFile(targetPath: string): Promise<string> {
    const sftp = await this.ensureSftp()
    return new Promise<string>((resolve, reject) => {
      sftp.readFile(targetPath, 'utf8', (error, data) => {
        if (error) {
          reject(error)
          return
        }
        resolve(typeof data === 'string' ? data : data.toString('utf8'))
      })
    })
  }

  async writeRemoteFile(targetPath: string, content: string): Promise<void> {
    const sftp = await this.ensureSftp()
    await new Promise<void>((resolve, reject) => {
      sftp.writeFile(targetPath, content, 'utf8', (error) => {
        if (error) {
          reject(error)
          return
        }
        resolve()
      })
    })
  }

  async uploadFile(localPath: string, remotePath: string, onProgress: (progress: number) => void): Promise<void> {
    const sftp = await this.ensureSftp()
    const info = await stat(localPath)
    const total = Math.max(info.size, 1)
    await new Promise<void>((resolve, reject) => {
      sftp.fastPut(localPath, remotePath, {
        step: (transferred) => onProgress(Math.min(99, Math.round((transferred / total) * 100)))
      }, (error) => {
        if (error) {
          reject(error)
          return
        }
        onProgress(100)
        resolve()
      })
    })
  }

  async downloadFile(remotePath: string, localPath: string, onProgress: (progress: number) => void): Promise<void> {
    const sftp = await this.ensureSftp()
    const attrs = await new Promise<{ size?: number }>((resolve, reject) => {
      sftp.stat(remotePath, (error, stats) => {
        if (error || !stats) {
          reject(error ?? new Error(`Failed to stat remote file: ${remotePath}`))
          return
        }
        resolve(stats)
      })
    })
    const total = Math.max(attrs.size ?? 1, 1)
    await new Promise<void>((resolve, reject) => {
      sftp.fastGet(remotePath, localPath, {
        step: (transferred) => onProgress(Math.min(99, Math.round((transferred / total) * 100)))
      }, (error) => {
        if (error) {
          reject(error)
          return
        }
        onProgress(100)
        resolve()
      })
    })
  }

  async refreshSystemMetrics(): Promise<SystemMetrics | undefined> {
    try {
      const raw = await this.execCommand(buildMetricsCommand())
      this.metrics = parseSystemMetrics(raw)
      return this.metrics
    } catch {
      return this.metrics
    }
  }

  private async ensureSftp(): Promise<SFTPWrapper> {
    if (this.sftp) {
      return this.sftp
    }

    return new Promise<SFTPWrapper>((resolve, reject) => {
      this.ssh.sftp((error, sftp) => {
        if (error || !sftp) {
          reject(error ?? new Error('Failed to open SFTP session'))
          return
        }
        this.sftp = sftp
        resolve(sftp)
      })
    })
  }

  private async readRemoteDirectory(targetPath: string): Promise<RemoteFileItem[]> {
    const sftp = await this.ensureSftp()
    const entries = await new Promise<FileEntry[]>((resolve, reject) => {
      sftp.readdir(targetPath, (error, list) => {
        if (error || !list) {
          reject(error ?? new Error(`Failed to read remote directory: ${targetPath}`))
          return
        }
        resolve(list)
      })
    })

    const rows = entries
      .filter((entry) => entry.filename !== '.' && entry.filename !== '..')
      .map((entry) => toRemoteFileItem(targetPath, entry))
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

  private async execCommand(command: string): Promise<string> {
    return new Promise<string>((resolve, reject) => {
      this.ssh.exec(command, (error, stream) => {
        if (error) {
          reject(error)
          return
        }

        let stdout = ''
        let stderr = ''

        stream.on('data', (chunk: Buffer) => {
          stdout += chunk.toString('utf8')
        })
        stream.stderr.on('data', (chunk: Buffer) => {
          stderr += chunk.toString('utf8')
        })
        stream.on('close', (code?: number) => {
          if (code && code !== 0 && stderr.trim()) {
            reject(new Error(stderr.trim()))
            return
          }
          resolve(stdout)
        })
      })
    })
  }

  private appendSystemMessage(message: string) {
    this.transcript += message
    this.onData(message)
  }
}
