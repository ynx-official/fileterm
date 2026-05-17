import { useState } from 'react'
import type { TransferTask } from '@termdock/core'
import { transferStatusText } from '../../app/app-utils'

export function TransferPopover({
  onClose,
  transfers
}: {
  onClose(): void
  transfers: TransferTask[]
}) {
  const [statusFilter, setStatusFilter] = useState<'running' | 'completed' | 'all'>('running')
  const [directionFilter, setDirectionFilter] = useState<'all' | 'download' | 'upload'>('all')
  const visibleTransfers = transfers
    .filter((transfer) => {
      if (statusFilter === 'running') {
        return transfer.status === 'running' || transfer.status === 'queued'
      }
      if (statusFilter === 'completed') {
        return transfer.status === 'done' || transfer.status === 'failed'
      }
      return true
    })
    .filter((transfer) => directionFilter === 'all' || transfer.direction === directionFilter)
    .slice(0, 24)

  return (
    <section className="transfer-popover">
      <div className="transfer-popover-head">
        <strong>传输详情</strong>
        <button className="icon-button" onClick={onClose} type="button">×</button>
      </div>
      <div className="transfer-filters">
        <div className="transfer-segments">
          <button className={statusFilter === 'running' ? 'active' : ''} onClick={() => setStatusFilter('running')} type="button">进行中</button>
          <button className={statusFilter === 'completed' ? 'active' : ''} onClick={() => setStatusFilter('completed')} type="button">已完成</button>
          <button className={statusFilter === 'all' ? 'active' : ''} onClick={() => setStatusFilter('all')} type="button">全部</button>
        </div>
        <div className="transfer-segments transfer-segments-sub">
          <button className={directionFilter === 'all' ? 'active' : ''} onClick={() => setDirectionFilter('all')} type="button">全部</button>
          <button className={directionFilter === 'download' ? 'active' : ''} onClick={() => setDirectionFilter('download')} type="button">下载</button>
          <button className={directionFilter === 'upload' ? 'active' : ''} onClick={() => setDirectionFilter('upload')} type="button">上传</button>
        </div>
      </div>
      <div className="transfer-popover-list">
        {visibleTransfers.length ? visibleTransfers.map((transfer) => (
          <div className={`transfer-row transfer-${transfer.status}`} key={transfer.id}>
            <div className="transfer-row-main">
              <strong title={transfer.name}>{transfer.name}</strong>
              <span>{transferStatusText(transfer)}</span>
            </div>
            <div className="transfer-row-meta">
              <span>{transfer.direction === 'upload' ? '上传' : '下载'}</span>
              <b>{transfer.progress}%</b>
            </div>
            <i className="transfer-progress"><b style={{ width: `${transfer.progress}%` }} /></i>
            {transfer.message ? <small title={transfer.message}>{transfer.message}</small> : null}
          </div>
        )) : (
          <div className="transfer-empty">暂无传输任务</div>
        )}
      </div>
    </section>
  )
}
