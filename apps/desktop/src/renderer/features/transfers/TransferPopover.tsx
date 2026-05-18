import { useState } from 'react'
import type { TransferTask } from '@termdock/core'
import { isActiveTransfer, isCompletedTransfer, transferStatusText } from '../../app/app-utils'
import { t } from '../../i18n'

export function TransferPopover({
  onCancelTransfer,
  onClose,
  transfers
}: {
  onCancelTransfer(transferId: string): void
  onClose(): void
  transfers: TransferTask[]
}) {
  const [statusFilter, setStatusFilter] = useState<'running' | 'completed' | 'all'>('running')
  const [directionFilter, setDirectionFilter] = useState<'all' | 'download' | 'upload'>('all')
  const visibleTransfers = transfers
    .filter((transfer) => {
      if (statusFilter === 'running') {
        return isActiveTransfer(transfer)
      }
      if (statusFilter === 'completed') {
        return isCompletedTransfer(transfer)
      }
      return true
    })
    .filter((transfer) => directionFilter === 'all' || transfer.direction === directionFilter)
    .slice(0, 24)

  return (
    <section className="transfer-popover">
      <div className="transfer-popover-head">
        <strong>{t.transferDetails}</strong>
        <button className="icon-button" onClick={onClose} type="button">×</button>
      </div>
      <div className="transfer-filters">
        <div className="transfer-segments">
          <button className={statusFilter === 'running' ? 'active' : ''} onClick={() => setStatusFilter('running')} type="button">{t.inProgress}</button>
          <button className={statusFilter === 'completed' ? 'active' : ''} onClick={() => setStatusFilter('completed')} type="button">{t.completed}</button>
          <button className={statusFilter === 'all' ? 'active' : ''} onClick={() => setStatusFilter('all')} type="button">{t.all}</button>
        </div>
        <div className="transfer-segments transfer-segments-sub">
          <button className={directionFilter === 'all' ? 'active' : ''} onClick={() => setDirectionFilter('all')} type="button">{t.all}</button>
          <button className={directionFilter === 'download' ? 'active' : ''} onClick={() => setDirectionFilter('download')} type="button">{t.download}</button>
          <button className={directionFilter === 'upload' ? 'active' : ''} onClick={() => setDirectionFilter('upload')} type="button">{t.upload}</button>
        </div>
      </div>
      <div className="transfer-popover-list">
        {visibleTransfers.length ? visibleTransfers.map((transfer) => (
          <div className={`transfer-row transfer-${transfer.status}`} key={transfer.id}>
            <div className="transfer-row-main">
              <strong title={transfer.name}>{transfer.name}</strong>
              <div className="transfer-row-inline">
                <span>{transferStatusText(transfer)}</span>
                {isActiveTransfer(transfer) ? (
                  <button className="transfer-cancel" onClick={() => onCancelTransfer(transfer.id)} type="button">{t.stop}</button>
                ) : null}
              </div>
            </div>
            <div className="transfer-row-meta">
              <span>{transfer.direction === 'upload' ? t.upload : t.download}</span>
              <b>{transfer.progress}%</b>
            </div>
            <i className="transfer-progress"><b style={{ width: `${transfer.progress}%` }} /></i>
            {transfer.message ? <small title={transfer.message}>{transfer.message}</small> : null}
          </div>
        )) : (
          <div className="transfer-empty">{t.noTransferTasks}</div>
        )}
      </div>
    </section>
  )
}
