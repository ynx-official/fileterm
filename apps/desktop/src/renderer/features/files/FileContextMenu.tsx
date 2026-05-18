import type { LocalFileItem, RemoteFileItem } from '@termdock/core'
import { ContextMenu } from '../common/ContextMenu'

export function FileContextMenu({
  item,
  onClose,
  onCopyPath,
  onDownload,
  onOpen,
  onRefresh,
  onUpload,
  pane,
  position
}: {
  item: LocalFileItem | RemoteFileItem | null
  onClose(): void
  onCopyPath(): void
  onDownload(): void
  onOpen(): void
  onRefresh(): void
  onUpload(): void
  pane: 'local' | 'remote'
  position: { x: number; y: number }
}) {
  const canDownload = pane === 'remote' && item?.type === 'file'
  const canUpload = pane === 'remote' || pane === 'local'
  const uploadLabel = pane === 'local' ? '上传到远程' : '上传...'
  const items = [
    { label: '刷新', action: onRefresh },
    { separator: true },
    { label: '打开', disabled: !item, action: onOpen },
    { separator: true },
    { label: '复制路径', disabled: !item, action: onCopyPath },
    ...(canDownload ? [{ separator: true }, { label: '下载', action: onDownload }] : []),
    ...(canUpload ? [{ separator: true }, { label: uploadLabel, action: onUpload }] : [])
  ]

  return (
    <ContextMenu
      className="file-context-menu"
      items={items}
      onClose={onClose}
      position={position}
    />
  )
}
