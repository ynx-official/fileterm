import type { ConnectionFormMode, ConnectionProfile, CreateProfileInput } from '@fileterm/core'
import type { FormEvent } from 'react'
import { ConnectionModal } from './ConnectionModal'

export function ConnectionFormHost({
  editingProfileId,
  errorMessage,
  form,
  groupOptions,
  isSubmitting,
  mode,
  profiles,
  setForm,
  standalone,
  onClearHostFingerprint,
  onClose,
  onSubmit
}: {
  editingProfileId: string | null
  errorMessage: string | null
  form: CreateProfileInput
  groupOptions: string[]
  isSubmitting?: boolean
  mode: ConnectionFormMode
  profiles: ConnectionProfile[]
  setForm(updater: CreateProfileInput | ((current: CreateProfileInput) => CreateProfileInput)): void
  standalone?: boolean
  onClearHostFingerprint(profile: ConnectionProfile): void
  onClose(): void
  onSubmit(event: FormEvent<HTMLFormElement>): void
}) {
  const editingProfile = editingProfileId ? (profiles.find((profile) => profile.id === editingProfileId) ?? null) : null

  const clearHostFingerprint = () => {
    if (!editingProfile) {
      return
    }
    onClearHostFingerprint(editingProfile)
    setForm((prev) => ({ ...prev, trustedHostFingerprint: '' }))
  }

  return (
    <ConnectionModal
      errorMessage={errorMessage}
      groupOptions={groupOptions}
      isSubmitting={isSubmitting}
      mode={mode}
      form={form}
      hasSavedPassword={editingProfile?.hasSavedPassword === true}
      profiles={profiles}
      setForm={setForm}
      onClearHostFingerprint={clearHostFingerprint}
      standalone={standalone}
      onSubmit={onSubmit}
      onClose={onClose}
    />
  )
}
