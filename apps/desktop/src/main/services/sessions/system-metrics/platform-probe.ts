import type { RemoteSystemPlatform, SystemMetricsExecutor } from './types.js'

export async function probeRemoteSystemPlatform(executor: SystemMetricsExecutor): Promise<RemoteSystemPlatform> {
  const posixPlatform = await probePosixPlatform(executor)
  if (posixPlatform !== 'unknown') {
    return posixPlatform
  }

  return probeWindowsPlatform(executor)
}

async function probePosixPlatform(executor: SystemMetricsExecutor): Promise<RemoteSystemPlatform> {
  try {
    const raw = await executor.exec(
      'sh -lc \'printf "__FILETERM_PROBE_START__\\n"; uname -s 2>/dev/null; if command -v busybox >/dev/null 2>&1; then busybox 2>&1 | head -n 1; fi; if [ -f /etc/openwrt_release ]; then printf "openwrt\\n"; fi; printf "__FILETERM_PROBE_END__\\n"\'',
      { allowNonZeroWithStdout: true }
    )
    const body = extractProbeBody(raw)
    if (!body) {
      return 'unknown'
    }
    const normalized = body.toLowerCase()
    if (normalized.includes('openwrt') || normalized.includes('busybox')) {
      return 'busybox'
    }
    if (normalized.includes('linux')) {
      return 'linux'
    }
    return 'unknown'
  } catch {
    return 'unknown'
  }
}

async function probeWindowsPlatform(executor: SystemMetricsExecutor): Promise<RemoteSystemPlatform> {
  const commands = [
    'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "[Environment]::OSVersion.Platform"',
    'pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "[Environment]::OSVersion.Platform"',
    'cmd /c ver'
  ]

  for (const command of commands) {
    try {
      const raw = await executor.exec(command, { allowNonZeroWithStdout: true })
      if (/windows|win32nt/i.test(raw)) {
        return 'windows'
      }
    } catch {
      // Try the next Windows shell candidate.
    }
  }

  return 'unknown'
}

function extractProbeBody(raw: string) {
  const start = raw.indexOf('__FILETERM_PROBE_START__')
  const end = raw.indexOf('__FILETERM_PROBE_END__')
  if (start === -1 || end === -1 || end <= start) {
    return ''
  }
  return raw.slice(start, end)
}
