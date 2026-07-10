import assert from 'node:assert/strict'
import test from 'node:test'
import { runPowerShellMetricsScript } from '../../src/main/services/sessions/system-metrics/windows-collector.ts'

const WINDOWS_METRICS_OUTPUT = [
  '__PLATFORM__windows',
  '__OS__Microsoft Windows Server 2022',
  '__HOSTNAME__WIN-COLLECTOR',
  '__FILETERM_METRICS_COMPLETE__'
].join('\r\n')

test('runs the bounded Windows fallback collector through PowerShell stdin', async () => {
  let stdinPayload = ''
  const raw = await runPowerShellMetricsScript({
    async exec(command, options, stdin) {
      assert.match(command, /^powershell /)
      assert.equal(options, undefined)
      stdinPayload = stdin ?? ''
      return WINDOWS_METRICS_OUTPUT
    }
  })

  assert.equal(raw, WINDOWS_METRICS_OUTPUT)
  assert.match(stdinPayload, /^\s*& \{/)
  assert.match(stdinPayload, /Get-CimInstance/)
  assert.match(stdinPayload, /Get-WmiObject/)
  assert.match(stdinPayload, /Wait-Job -Job \$job -Timeout 2/)
  assert.match(stdinPayload, /wmic\.exe/)
  assert.match(stdinPayload, /WaitForExit\(2000\)/)
  assert.match(stdinPayload, /ipconfig\.exe/)
  assert.match(stdinPayload, /tasklist\.exe/)
  assert.match(stdinPayload, /__FILETERM_METRICS_COMPLETE__"\s*\n}/)
})

test('falls back to pwsh when Windows PowerShell does not complete', async () => {
  const commands: string[] = []
  const raw = await runPowerShellMetricsScript({
    async exec(command) {
      commands.push(command)
      if (command.startsWith('powershell ')) {
        return '__PLATFORM__windows\r\n'
      }
      return WINDOWS_METRICS_OUTPUT
    }
  })

  assert.equal(raw, WINDOWS_METRICS_OUTPUT)
  assert.equal(commands.length, 2)
  assert.match(commands[1] ?? '', /^pwsh /)
})
