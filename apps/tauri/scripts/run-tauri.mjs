import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const env = { ...process.env }
const pathKey = Object.keys(env).find((key) => key.toLowerCase() === 'path') ?? 'PATH'
const pathEntries = (env[pathKey] ?? '').split(path.delimiter).filter(Boolean)

function prependExecutableDirectory(directory, executable) {
  if (!directory || !fs.existsSync(path.join(directory, executable))) {
    return
  }

  const normalizedDirectory = path.resolve(directory).toLowerCase()
  if (!pathEntries.some((entry) => path.resolve(entry).toLowerCase() === normalizedDirectory)) {
    pathEntries.unshift(directory)
  }
}

const cargoHome = env.CARGO_HOME ? path.resolve(env.CARGO_HOME) : path.join(os.homedir(), '.cargo')
prependExecutableDirectory(path.join(cargoHome, 'bin'), process.platform === 'win32' ? 'cargo.exe' : 'cargo')

if (process.platform === 'win32') {
  prependExecutableDirectory(path.join(env.LOCALAPPDATA ?? '', 'bin', 'NASM'), 'nasm.exe')
}

env[pathKey] = pathEntries.join(path.delimiter)

const tauriCli = require.resolve('@tauri-apps/cli/tauri.js')
const child = spawn(process.execPath, [tauriCli, ...process.argv.slice(2)], {
  env,
  stdio: 'inherit'
})

child.on('error', (error) => {
  console.error(`[FileTerm] failed to start the local Tauri CLI: ${error.message}`)
  process.exitCode = 1
})

child.on('exit', (code) => {
  process.exitCode = code ?? 1
})
