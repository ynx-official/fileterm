import type { ConnectionProfile, TransferTask } from '@termdock/core'

export const seedProfiles: ConnectionProfile[] = [
  {
    id: 'profile-ssh-prod',
    type: 'ssh',
    name: 'prod-web-01',
    host: '10.0.0.21',
    port: 22,
    username: 'root',
    authType: 'privateKey',
    privateKeyPath: '~/.ssh/id_ed25519',
    group: 'Production',
    sftpEnabled: true,
    remotePath: '/srv/www'
  },
  {
    id: 'profile-ssh-nas',
    type: 'ssh',
    name: 'nas-storage',
    host: '10.0.0.44',
    port: 22,
    username: 'admin',
    authType: 'password',
    group: 'Staging',
    sftpEnabled: true,
    remotePath: '/volume1'
  },
  {
    id: 'profile-ftp-archive',
    type: 'ftp',
    name: 'archive-ftp',
    host: 'ftp.example.net',
    port: 21,
    username: 'deploy',
    secure: false,
    group: 'FTP Sites',
    remotePath: '/incoming'
  }
]

export const seedTransfers: TransferTask[] = []
