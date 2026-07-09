import { parseSystemMetrics } from './parser.js'
import { buildPosixMetricsCommand } from './posix-script.js'
import type { SystemMetricsCollector } from './types.js'

export const linuxCollector: SystemMetricsCollector = {
  platform: 'linux',
  async collect(executor) {
    const raw = await executor.exec('sh', { allowNonZeroWithStdout: true }, `${buildPosixMetricsCommand('linux')}\n`)
    return parseSystemMetrics(raw, 'linux')
  }
}
