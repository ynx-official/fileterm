import { parseSystemMetrics } from './parser.js'
import { buildPosixMetricsCommand } from './posix-script.js'
import type { SystemMetricsCollector } from './types.js'

export const busyboxCollector: SystemMetricsCollector = {
  platform: 'busybox',
  async collect(executor) {
    const raw = await executor.exec('sh', { allowNonZeroWithStdout: true }, `${buildPosixMetricsCommand('busybox')}\n`)
    return parseSystemMetrics(raw, 'busybox')
  }
}
