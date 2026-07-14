import { useEffect, useState } from 'react'
import { ConfirmActionDialog } from '../common/ConfirmActionDialog'

export function SshKeyNoteDialog({
  errorMessage,
  initialNote = '',
  isSubmitting,
  mode,
  onClose,
  onSubmit
}: {
  errorMessage?: string | null
  initialNote?: string
  isSubmitting: boolean
  mode: 'import' | 'edit'
  onClose(): void
  onSubmit(note: string): void
}) {
  const [note, setNote] = useState(initialNote)
  const normalizedNote = note.trim()

  useEffect(() => {
    setNote(initialNote)
  }, [initialNote, mode])

  return (
    <ConfirmActionDialog
      confirmDisabled={!normalizedNote}
      confirmLabel={mode === 'import' ? '选择私钥文件' : '保存备注'}
      confirmVariant="primary"
      description={
        <label className="ssh-key-note-dialog__field">
          <span>备注信息</span>
          <input
            autoFocus
            maxLength={120}
            placeholder="例如：生产服务器部署密钥"
            value={note}
            onChange={(event) => setNote(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && normalizedNote && !isSubmitting) {
                onSubmit(normalizedNote)
              }
            }}
          />
          <small>备注用于在连接表单中优先识别密钥，不能为空。</small>
        </label>
      }
      errorMessage={errorMessage}
      isSubmitting={isSubmitting}
      onClose={onClose}
      onConfirm={() => {
        if (normalizedNote) onSubmit(normalizedNote)
      }}
      title={mode === 'import' ? '导入 SSH 私钥' : '修改密钥备注'}
    />
  )
}
