import { useEffect, useState } from 'react'
import type { FileContentSnapshot } from '@termdock/core'
import { t } from '../../i18n'

export function FileEditorModal({
  errorMessage,
  file,
  onClose,
  onSave
}: {
  errorMessage: string | null
  file: FileContentSnapshot
  onClose(): void
  onSave(content: string): void
}) {
  const [content, setContent] = useState(file.content)

  useEffect(() => {
    setContent(file.content)
  }, [file.content, file.path])

  return (
    <div className="modal-backdrop">
      <div className="modal-card file-editor-modal">
        <div className="modal-header">
          <span>{file.source === 'remote' ? t.editRemoteFile : t.editLocalFile} · {file.name}</span>
          <button className="icon-button" onClick={onClose} type="button">×</button>
        </div>
        <div className="file-editor-path" title={file.path}>{file.path}</div>
        <textarea
          className="file-editor-textarea"
          spellCheck={false}
          value={content}
          onChange={(event) => setContent(event.target.value)}
        />
        {errorMessage ? <div className="modal-error">{errorMessage}</div> : null}
        <div className="form-actions">
          <button className="flat-button" onClick={onClose} type="button">{t.cancel}</button>
          <button className="primary-button" onClick={() => onSave(content)} type="button">{t.save}</button>
        </div>
      </div>
    </div>
  )
}
