import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createConnection, createServer } from 'node:net'
import { access, chmod, lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { LiveSshSessionController } from '../../dist-electron/main/services/sessions/ssh-session-controller.js'

test('SFTP resumes both directions and preserves a destination symlink', async (t) => {
  const fixture = await startSshdFixture(t)
  if (!fixture) return

  const controller = new LiveSshSessionController(
    'sftp-test',
    {
      id: 'sftp-profile',
      type: 'ssh',
      name: 'SFTP integration',
      host: '127.0.0.1',
      port: fixture.port,
      username: os.userInfo().username,
      authType: 'privateKey',
      privateKeyPath: fixture.clientKey,
      group: 'test',
      sftpEnabled: true,
      remotePath: fixture.remoteDir,
      enableExecChannel: true
    },
    async (request) => request.kind === 'host-verification'
      ? { kind: 'host-verification', decision: 'accept-once' }
      : { kind: 'credentials', canceled: true },
    async () => undefined,
    () => undefined,
    () => undefined,
    () => undefined,
    () => undefined
  )
  t.after(() => controller.disconnect())
  await controller.connect()

  const sourcePath = path.join(fixture.localDir, 'source.bin')
  const destinationPath = path.join(fixture.remoteDir, 'target.bin')
  const partialPath = `${destinationPath}.fileterm-part`
  const contents = Buffer.from('hello-sftp-resume')
  await writeFile(sourcePath, contents)
  await writeFile(partialPath, contents.subarray(0, 6))

  await controller.uploadFile(sourcePath, partialPath, () => undefined, { resumeOffset: 6 })
  assert.deepEqual(await readFile(partialPath), contents)
  await controller.replaceRemoteFile(partialPath, destinationPath)
  assert.deepEqual(await readFile(destinationPath), contents)

  const localDownload = path.join(fixture.localDir, 'download.bin')
  await writeFile(localDownload, contents.subarray(0, 5))
  await controller.downloadFile(destinationPath, localDownload, () => undefined, { resumeOffset: 5 })
  assert.deepEqual(await readFile(localDownload), contents)

  const emptySource = path.join(fixture.localDir, 'empty.bin')
  const emptyRemote = path.join(fixture.remoteDir, 'empty.bin.fileterm-part')
  const emptyDownload = path.join(fixture.localDir, 'empty-download.bin')
  await writeFile(emptySource, '')
  await controller.uploadFile(emptySource, emptyRemote, () => undefined)
  await controller.downloadFile(emptyRemote, emptyDownload, () => undefined)
  assert.equal((await lstat(emptyDownload)).size, 0)
  await assert.rejects(
    () => controller.uploadFile(emptySource, emptyRemote, () => undefined, { resumeOffset: 1 }),
    /断点大于源文件/
  )

  const symlinkTarget = path.join(fixture.remoteDir, 'symlink-target.bin')
  const symlinkPath = path.join(fixture.remoteDir, 'symlink.bin')
  const symlinkPart = `${symlinkPath}.fileterm-part`
  await writeFile(symlinkTarget, 'old')
  await symlink(symlinkTarget, symlinkPath)
  await writeFile(symlinkPart, 'new-through-link')
  await controller.replaceRemoteFile(symlinkPart, symlinkPath)
  assert.equal((await lstat(symlinkPath)).isSymbolicLink(), true)
  assert.equal(await readFile(symlinkTarget, 'utf8'), 'new-through-link')
})

async function startSshdFixture(t) {
  if (process.platform === 'win32') {
    t.skip('本地 OpenSSH sshd 夹具仅在 macOS/Linux 运行')
    return null
  }
  const sshdPath = '/usr/sbin/sshd'
  const sshKeygenPath = '/usr/bin/ssh-keygen'
  try {
    await Promise.all([access(sshdPath), access(sshKeygenPath)])
  } catch {
    t.skip('系统未安装 /usr/sbin/sshd 或 /usr/bin/ssh-keygen')
    return null
  }
  const baseDir = await mkdtemp(path.join(os.tmpdir(), 'fileterm-sftp-test-'))
  const localDir = path.join(baseDir, 'local')
  const remoteDir = path.join(baseDir, 'remote')
  await mkdir(localDir, { recursive: true })
  await mkdir(remoteDir, { recursive: true })
  const hostKey = path.join(baseDir, 'host-key')
  const clientKey = path.join(baseDir, 'client-key')
  const authorizedKeys = path.join(baseDir, 'authorized_keys')
  try {
    execFileSync(sshKeygenPath, ['-q', '-t', 'ed25519', '-N', '', '-f', hostKey])
    execFileSync(sshKeygenPath, ['-q', '-t', 'ed25519', '-N', '', '-f', clientKey])
  } catch (error) {
    await rm(baseDir, { recursive: true, force: true })
    t.skip(`无法生成 OpenSSH 测试密钥：${error instanceof Error ? error.message : String(error)}`)
    return null
  }
  await writeFile(authorizedKeys, await readFile(`${clientKey}.pub`))
  await chmod(authorizedKeys, 0o600)
  const port = await reservePort()
  const configPath = path.join(baseDir, 'sshd_config')
  const pidPath = path.join(baseDir, 'sshd.pid')
  await writeFile(configPath, [
    `Port ${port}`,
    'ListenAddress 127.0.0.1',
    `HostKey ${hostKey}`,
    `PidFile ${pidPath}`,
    `AuthorizedKeysFile ${authorizedKeys}`,
    'StrictModes no',
    'PasswordAuthentication no',
    'KbdInteractiveAuthentication no',
    'ChallengeResponseAuthentication no',
    'PubkeyAuthentication yes',
    'UsePAM no',
    'UseDNS no',
    'LogLevel ERROR',
    'Subsystem sftp internal-sftp'
  ].join('\n'))

  const sshd = spawn(sshdPath, ['-D', '-e', '-f', configPath], {
    stdio: ['ignore', 'ignore', 'pipe']
  })
  let stderr = ''
  sshd.stderr.on('data', (chunk) => {
    stderr += chunk.toString('utf8')
  })
  try {
    await waitForPort(port, sshd)
  } catch (error) {
    sshd.kill('SIGTERM')
    await rm(baseDir, { recursive: true, force: true })
    t.skip(`本地 sshd 无法启动：${stderr.trim() || (error instanceof Error ? error.message : String(error))}`)
    return null
  }

  t.after(async () => {
    sshd.kill('SIGTERM')
    await new Promise((resolve) => {
      if (sshd.exitCode !== null) return resolve()
      sshd.once('exit', resolve)
      setTimeout(resolve, 1_000).unref()
    })
    await rm(baseDir, { recursive: true, force: true })
  })
  return { baseDir, clientKey, localDir, port, remoteDir }
}

async function reservePort() {
  const server = createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

async function waitForPort(port, processHandle) {
  const deadline = Date.now() + 5_000
  while (Date.now() < deadline) {
    if (processHandle.exitCode !== null) {
      throw new Error(`sshd exited with ${processHandle.exitCode}`)
    }
    const connected = await new Promise((resolve) => {
      const socket = createConnection({ host: '127.0.0.1', port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => resolve(false))
    })
    if (connected) return
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  throw new Error('sshd startup timeout')
}
