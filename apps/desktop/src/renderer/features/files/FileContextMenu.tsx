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
  const canUpload = pane === 'remote'

  return (
    <ContextMenu
      className="file-context-menu"
      items={[
        { label: '刷新', action: onRefresh },
        { separator: true },
        { label: '打开', disabled: !item, action: onOpen },
        { label: '打开方式', disabled: true },
        { label: '选择文本编辑器', disabled: true },
        { separator: true },
        { label: '复制路径', disabled: !item, action: onCopyPath },
        { separator: true },
        { label: '下载', disabled: !canDownload, action: onDownload },
        { label: '上传...', disabled: !canUpload, action: onUpload },
        { separator: true },
        { label: '打包传输', disabled: true },
        { separator: true },
        { label: '新建', disabled: true },
        { separator: true },
        { label: '重命名', disabled: true },
        { label: '删除', disabled: true, danger: true },
        { label: '快速删除 (rm命令)', disabled: true, danger: true },
        { separator: true },
        { label: '文件权限...', disabled: true }
      ]}
      onClose={onClose}
      position={position}
    />
  )
}
