import type { LocalFileItem, RemoteFileItem } from '@termdock/core'
import { t } from '../../i18n'
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
  const uploadLabel = pane === 'local' ? t.uploadToRemote : t.uploadMore
  const items = [
    { label: t.refresh, action: onRefresh },
    { separator: true },
    { label: t.open, disabled: !item, action: onOpen },
    { separator: true },
    { label: t.copyPath, disabled: !item, action: onCopyPath },
    ...(canDownload ? [{ separator: true }, { label: t.download, action: onDownload }] : []),
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
