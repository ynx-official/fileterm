import type { FileTermDesktopApi, TransferTask, WorkspaceTab } from '@fileterm/core'
import { TransferCenter } from './TransferCenter'

export function TransferCenterHost({
  activeProfileId,
  activeTabId,
  desktopApi,
  fullWidth,
  initialTransfers,
  isPending,
  onError,
  sessionTabs,
  visible
}: {
  activeProfileId?: string
  activeTabId: string | null
  desktopApi?: FileTermDesktopApi
  fullWidth: boolean
  initialTransfers: TransferTask[]
  isPending: boolean
  onError(scope: string, err: unknown): void
  sessionTabs: WorkspaceTab[]
  visible: boolean
}) {
  return (
    <TransferCenter
      activeProfileId={activeProfileId}
      activeTabId={activeTabId}
      desktopApi={desktopApi}
      fullWidth={fullWidth}
      initialTransfers={initialTransfers}
      isPending={isPending}
      onError={onError}
      sessionTabs={sessionTabs.map((tab) => ({
        id: tab.id,
        profileId: tab.profileId
      }))}
      visible={visible}
    />
  )
}
