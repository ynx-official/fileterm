import type { ConnectionProfile } from '@termdock/core'
import type { MouseEvent } from 'react'

export function ConnectionManagerModal({
  profiles,
  onClose,
  onCreate,
  onDelete,
  onEdit,
  onOpen,
  standalone = false
}: {
  profiles: ConnectionProfile[]
  onClose(): void
  onCreate(): void
  onDelete(event: MouseEvent<HTMLButtonElement>, profileId: string): void
  onEdit(profile: ConnectionProfile): void
  onOpen(profileId: string): void
  standalone?: boolean
}) {
  const content = (
    <div className={`modal-card manager-modal ${standalone ? 'standalone' : ''}`}>
      <div className="modal-header">
        <span>连接管理器</span>
        {!standalone ? <button className="icon-button" onClick={onClose} type="button">×</button> : null}
      </div>
      <div className="manager-toolbar">
        <button className="primary-button" type="button" onClick={onCreate}>新建连接</button>
      </div>
      <div className="manager-table">
        <div className="manager-head">
          <span>名称</span>
          <span>主机</span>
          <span>端口</span>
          <span>用户</span>
          <span>类型</span>
          <span>备注</span>
          <span>操作</span>
        </div>
        {profiles.map((profile) => (
          <div
            className="manager-row"
            key={profile.id}
            onDoubleClick={() => onOpen(profile.id)}
            onClick={() => onOpen(profile.id)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                onOpen(profile.id)
              }
            }}
            role="button"
            tabIndex={0}
          >
            <span>{profile.name}</span>
            <span>{profile.host}</span>
            <span>{profile.port}</span>
            <span>{profile.username}</span>
            <span>{profile.type.toUpperCase()}</span>
            <span>{profile.note || '/'}</span>
            <span className="manager-actions">
              <button
                className="flat-button compact"
                type="button"
                onClick={(event) => {
                  event.stopPropagation()
                  onEdit(profile)
                }}
              >
                编辑
              </button>
              <button
                className="flat-button compact danger"
                type="button"
                onClick={(event) => onDelete(event, profile.id)}
              >
                删除
              </button>
            </span>
          </div>
        ))}
      </div>
    </div>
  )

  if (standalone) {
    return <div className="manager-window">{content}</div>
  }

  return (
    <div className="modal-backdrop">
      {content}
    </div>
  )
}
