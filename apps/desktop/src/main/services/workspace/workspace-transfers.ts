import { randomUUID } from 'node:crypto'
import type { TransferTask } from '@termdock/core'

export class WorkspaceTransfersState {
  private readonly transfers: TransferTask[]

  constructor(seedTransfers: TransferTask[]) {
    this.transfers = [...seedTransfers]
  }

  list() {
    return [...this.transfers]
  }

  queueUploads(fileNames: string[]) {
    const queued = fileNames.map((name) => ({
      id: randomUUID(),
      direction: 'upload' as const,
      name,
      progress: 0,
      status: 'queued' as const
    }))

    this.transfers.unshift(...queued)
  }

  add(direction: TransferTask['direction'], name: string) {
    const queuedIndex = this.transfers.findIndex((transfer) => (
      transfer.direction === direction
      && transfer.name === name
      && transfer.status === 'queued'
    ))
    if (queuedIndex !== -1) {
      const queuedTransfer = this.transfers[queuedIndex]
      this.transfers[queuedIndex] = {
        ...queuedTransfer,
        progress: 0,
        status: 'running',
        message: undefined,
        speed: undefined
      }
      return queuedTransfer.id
    }

    const transferId = randomUUID()
    this.transfers.unshift({
      id: transferId,
      direction,
      name,
      progress: 0,
      status: 'running'
    })
    return transferId
  }

  update(
    transferId: string,
    patch: Partial<Pick<TransferTask, 'progress' | 'status' | 'message' | 'speed'>>
  ) {
    const index = this.transfers.findIndex((transfer) => transfer.id === transferId)
    if (index === -1) {
      return
    }

    const current = this.transfers[index]
    if (
      (current.status === 'done' || current.status === 'failed' || current.status === 'canceled')
      && (patch.status === 'running' || patch.status === 'queued')
    ) {
      return
    }

    this.transfers[index] = {
      ...current,
      ...patch
    }
  }

  get(transferId: string) {
    return this.transfers.find((transfer) => transfer.id === transferId)
  }
}
