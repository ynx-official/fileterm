import { useMemo, useRef, useState } from 'react'
import type { ConnectionImportConflictStrategy, ConnectionImportPlan } from '@fileterm/core'
import { CloseButton } from '../common/CloseButton'

export function ConnectionImportPreviewModal({
  plan,
  onClose,
  onCommit
}: {
  plan: ConnectionImportPlan
  onClose(): void
  onCommit(selectedItemIds: string[], conflictStrategy: ConnectionImportConflictStrategy): Promise<void>
}) {
  const readyIds = useMemo(
    () => plan.items.filter((item) => item.status === 'ready' && item.id).map((item) => item.id!),
    [plan]
  )
  const [selected, setSelected] = useState(() => new Set(readyIds))
  const [strategy, setStrategy] = useState<ConnectionImportConflictStrategy>('skip')
  const [isSubmitting, setIsSubmitting] = useState(false)
  const submittingRef = useRef(false)

  const commit = async () => {
    if (submittingRef.current || !selected.size) return
    submittingRef.current = true
    setIsSubmitting(true)
    try {
      await onCommit([...selected], strategy)
    } finally {
      submittingRef.current = false
      setIsSubmitting(false)
    }
  }

  const toggle = (id: string) =>
    setSelected((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  return (
    <div className="modal-backdrop connection-import-preview-backdrop" onClick={isSubmitting ? undefined : onClose}>
      <section className="modal-card connection-import-preview" onClick={(event) => event.stopPropagation()}>
        <header className="connection-manager-header">
          <span className="connection-manager-title">
            <span className="material-symbols-outlined">preview</span>
            <span>导入预览</span>
          </span>
          <CloseButton disabled={isSubmitting} onClick={onClose} />
        </header>
        <p className="connection-import-preview-hint">
          密码、私钥和代理密码不会显示在此预览中；它们只在确认后由主进程写入本地 secret storage。
        </p>
        <label className="connection-import-strategy">
          发现重复连接时
          <select
            disabled={isSubmitting}
            value={strategy}
            onChange={(event) => setStrategy(event.target.value as ConnectionImportConflictStrategy)}
          >
            <option value="skip">跳过重复项</option>
            <option value="overwrite">覆盖连接信息（保留未提供的凭据）</option>
            <option value="create">另存为新的连接</option>
          </select>
        </label>
        <div className="connection-import-list">
          {plan.items.map((item) => (
            <label
              key={item.id ?? `${item.sourceLabel}-${item.name}`}
              className={`connection-import-item is-${item.status}`}
            >
              <input
                disabled={isSubmitting || item.status !== 'ready' || !item.id}
                type="checkbox"
                checked={Boolean(item.id && selected.has(item.id))}
                onChange={() => item.id && toggle(item.id)}
              />
              <span className="connection-import-item-main">
                <strong>{item.name}</strong>
                <small>
                  {item.type.toUpperCase()} · {item.host ?? item.sourceLabel ?? '—'}
                  {item.port ? `:${item.port}` : ''}
                  {item.username ? ` · ${item.username}` : ''}
                </small>
                {item.unsupportedFields?.length ? <small>忽略字段：{item.unsupportedFields.join(', ')}</small> : null}
              </span>
              <span className="connection-import-item-status">
                {item.status === 'invalid' ? item.reason : item.conflictProfileId ? '检测到重复项' : '可导入'}
              </span>
            </label>
          ))}
        </div>
        <footer className="connection-import-actions">
          <span>
            已选择 {selected.size} / {readyIds.length} 项
          </span>
          <button disabled={isSubmitting} type="button" onClick={onClose}>
            取消
          </button>
          <button
            className="primary-button compact"
            disabled={!selected.size || isSubmitting}
            type="button"
            onClick={() => void commit()}
          >
            {isSubmitting ? <span aria-hidden="true" className="button-spinner" /> : null}
            <span>{isSubmitting ? '正在导入…' : '确认导入'}</span>
          </button>
        </footer>
      </section>
    </div>
  )
}
