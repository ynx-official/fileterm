import type { ConnectionProfile } from '@termdock/core'
import { t } from '../../i18n'
import { AppIcon } from '../common/AppIcon'

export function HomeWorkspace({
  profiles,
  isDesktopRuntime,
  onCreate,
  onOpen
}: {
  profiles: ConnectionProfile[]
  isDesktopRuntime: boolean
  onCreate(): void
  onOpen(profileId: string): void
}) {
  return (
    <section className="home-workspace">
      <div className="quick-panel">
        <div className="quick-header">
          <strong>{t.quickConnect}</strong>
          <div>
            <button className="flat-button" type="button" disabled={!isDesktopRuntime} onClick={onCreate}>{t.newConnection}</button>
          </div>
        </div>
        <div className="quick-list">
          {profiles.map((profile) => (
            <div
              className="quick-row"
              key={profile.id}
              onClick={() => onOpen(profile.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  onOpen(profile.id)
                }
              }}
              role="button"
              tabIndex={0}
            >
              <span className="host-icon"><AppIcon name="server" /></span>
              <strong>{profile.name}</strong>
              <span>{profile.note || '/'}</span>
              <span>{profile.username}</span>
              <small>{profile.type.toUpperCase()}</small>
            </div>
          ))}
        </div>
      </div>
    </section>
  )
}
