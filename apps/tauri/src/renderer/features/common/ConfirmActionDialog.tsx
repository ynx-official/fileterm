import { createPortal } from 'react-dom'
import { Children, cloneElement, isValidElement, useEffect, useId, useRef, type ReactNode } from 'react'
import { t } from '../../i18n'

export function ConfirmActionDialog({
  cancelLabel = t.cancel,
  confirmLabel,
  confirmDisabled = false,
  confirmVariant = 'danger',
  className,
  description,
  errorMessage,
  extraActions = null,
  isSubmitting = false,
  onClose,
  onConfirm,
  title
}: {
  cancelLabel?: string
  confirmLabel: string
  confirmDisabled?: boolean
  confirmVariant?: 'danger' | 'primary'
  className?: string
  description: ReactNode
  errorMessage?: string | null
  extraActions?: ReactNode
  isSubmitting?: boolean
  onClose(): void
  onConfirm(): void
  title: string
}) {
  const titleId = useId()
  const descriptionId = useId()
  const cancelButtonRef = useRef<HTMLButtonElement>(null)
  const confirmButtonClassName =
    confirmVariant === 'primary'
      ? 'confirm-action-dialog__button confirm-action-dialog__button--primary'
      : 'confirm-action-dialog__button confirm-action-dialog__button--danger'
  const cancelButtonClassName = 'confirm-action-dialog__button confirm-action-dialog__button--secondary'
  const guardedExtraActions = Children.map(extraActions, (action) =>
    isValidElement<{ disabled?: boolean }>(action)
      ? cloneElement(action, { disabled: isSubmitting || action.props.disabled })
      : action
  )

  useEffect(() => {
    cancelButtonRef.current?.focus()
  }, [])

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || isSubmitting) return
      event.preventDefault()
      event.stopPropagation()
      onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isSubmitting, onClose])

  const dialog = (
    <div className="modal-backdrop">
      <div
        className={`modal-card confirm-action-dialog${className ? ` ${className}` : ''}`}
        aria-busy={isSubmitting}
        aria-describedby={descriptionId}
        aria-labelledby={titleId}
        aria-modal="true"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
      >
        <div className="confirm-action-dialog__header">
          <div className="confirm-action-dialog__title" id={titleId}>
            {title}
          </div>
          <div className="confirm-action-dialog__description" id={descriptionId}>
            {description}
          </div>
          {errorMessage ? (
            <div className="modal-error" role="alert">
              {errorMessage}
            </div>
          ) : null}
        </div>
        <div className="form-actions confirm-action-dialog__footer">
          <button
            ref={cancelButtonRef}
            className={cancelButtonClassName}
            disabled={isSubmitting}
            onClick={onClose}
            type="button"
          >
            {cancelLabel}
          </button>
          {guardedExtraActions}
          <button
            className={confirmButtonClassName}
            disabled={isSubmitting || confirmDisabled}
            onClick={onConfirm}
            type="button"
          >
            {isSubmitting ? <span aria-hidden="true" className="button-spinner" /> : null}
            <span>{confirmLabel}</span>
          </button>
        </div>
      </div>
    </div>
  )

  if (typeof document === 'undefined') {
    return dialog
  }

  return createPortal(dialog, document.body)
}
