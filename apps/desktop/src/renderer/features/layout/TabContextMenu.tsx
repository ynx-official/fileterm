import type { WorkspaceTab } from '@termdock/core'
import { ContextMenu } from '../common/ContextMenu'

export function TabContextMenu({
  canConnectAll,
  canCloseAll,
  canCloseCurrent,
  canCloseOthers,
  isSessionTab,
  onAction,
  onClose,
  position,
  tabStatus
}: {
  canConnectAll: boolean
  canCloseAll: boolean
  canCloseCurrent: boolean
  canCloseOthers: boolean
  isSessionTab: boolean
  onAction(action: 'copy' | 'connect' | 'connectAll' | 'disconnect' | 'close' | 'closeOthers' | 'closeAll'): void
  onClose(): void
  position: { x: number; y: number }
  tabStatus: WorkspaceTab['status'] | null
}) {
  const canDisconnect = isSessionTab && tabStatus === 'connected'
  const canConnect = isSessionTab && (tabStatus === 'idle' || tabStatus === 'error' || tabStatus === 'closed')

  return (
    <ContextMenu
      className="tab-context-menu"
      items={[
        { label: '复制标签', action: () => onAction('copy') },
        { separator: true },
        { label: '连接', disabled: !canConnect, action: () => onAction('connect') },
        { label: '连接全部', disabled: !isSessionTab || !canConnectAll, action: () => onAction('connectAll') },
        { separator: true },
        { label: '断开', disabled: !canDisconnect, action: () => onAction('disconnect') },
        { separator: true },
        { label: '关闭', disabled: !canCloseCurrent, action: () => onAction('close') },
        { separator: true },
        { label: '关闭其他', disabled: !canCloseOthers, action: () => onAction('closeOthers') },
        { label: '关闭全部', disabled: !canCloseAll, action: () => onAction('closeAll') }
      ]}
      onClose={onClose}
      position={position}
    />
  )
}
