import type { TransferTask } from '@termdock/core'
import { runningTransfers } from '../../app/app-utils'
import { t } from '../../i18n'

export function TransferBar({
  activeCount,
  isPending,
  onOpen,
  transfers
}: {
  activeCount: number
  isPending: boolean
  onOpen(): void
  transfers: TransferTask[]
}) {
  return (
    <footer className="transfer-strip">
      <strong>{t.transferTasks}</strong>
      <button className="transfer-summary-button" onClick={onOpen} type="button">
        {isPending ? t.updating : `${activeCount || runningTransfers(transfers)} ${t.runningTasks}`}
      </button>
    </footer>
  )
}
