import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'
import test from 'node:test'
import { WorkspaceSessionRuntime } from '../../dist-electron/main/services/workspace/workspace-session-runtime.js'
import { WorkspaceWindowRegistry } from '../../dist-electron/main/services/windows/workspace-window-registry.js'

class FakeWebContents extends EventEmitter {
  constructor(id) {
    super()
    this.id = id
    this.destroyed = false
    this.sent = []
    this.mainFrame = {
      detached: false,
      isDestroyed: () => this.destroyed
    }
  }

  isDestroyed() {
    return this.destroyed
  }

  destroy() {
    this.destroyed = true
    this.emit('destroyed')
  }

  send(channel, payload) {
    this.sent.push({ channel, payload })
  }
}

class FakeBrowserWindow extends EventEmitter {
  constructor(id) {
    super()
    this.webContents = new FakeWebContents(id)
    this.shown = false
    this.focused = false
    this.minimized = false
    this.restored = false
    this.closed = false
  }

  isDestroyed() {
    return this.closed
  }

  isMinimized() {
    return this.minimized
  }

  restore() {
    this.minimized = false
    this.restored = true
  }

  show() {
    this.shown = true
  }

  focus() {
    this.focused = true
  }

  close() {
    const event = {
      defaultPrevented: false,
      preventDefault() {
        this.defaultPrevented = true
      }
    }
    this.emit('close', event)
    if (event.defaultPrevented || this.closed) return
    this.closed = true
    this.webContents.destroy()
    this.emit('closed')
  }
}

function createRegistry() {
  const mainWindow = new FakeBrowserWindow(1)
  const detachedWindows = []
  const claims = []
  const releases = []
  const registry = new WorkspaceWindowRegistry({
    getMainWindow: () => mainWindow,
    listTabIds: () => ['tab-a'],
    createDetachedWindow: () => {
      const window = new FakeBrowserWindow(100 + detachedWindows.length)
      detachedWindows.push(window)
      return window
    },
    claimTabRenderer: (tabId, sender) => claims.push({ tabId, senderId: sender.id }),
    releaseTabRenderer: (tabId, sender) => releases.push({ tabId, senderId: sender.id }),
    broadcastPlacements: () => undefined,
    isQuitting: () => false
  })
  registry.registerMainWindow(mainWindow)
  return { claims, detachedWindows, mainWindow, registry, releases }
}

function createRuntime() {
  return new WorkspaceSessionRuntime({
    getSnapshot: async () => ({
      profiles: [],
      folders: [],
      commandFolders: [],
      commandTemplates: [],
      tabs: [],
      activeTabId: null,
      transfers: [],
      sessions: {}
    }),
    getTabStatus: () => 'connected',
    resolveProfile: async () => null,
    rememberTrustedHostFingerprint: async () => undefined,
    resolveSshKey: async () => {
      throw new Error('not used')
    },
    setSshKeyPassphrase: async () => undefined
  })
}

test('detached tab ownership moves only after its renderer claims the tab', () => {
  const { claims, detachedWindows, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  assert.deepEqual(registry.listPlacements(), [{ tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main' }])

  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)

  assert.deepEqual(claims, [{ tabId: 'tab-a', senderId: detached.webContents.id }])
  assert.equal(detached.shown, true)
  assert.equal(detached.focused, true)
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session' }
  ])
})

test('closing a detached window returns the tab to the main renderer', () => {
  const { claims, detachedWindows, mainWindow, registry, releases } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)
  mainWindow.minimized = true

  detached.close()

  assert.equal(detached.closed, true)
  assert.equal(mainWindow.restored, true)
  assert.equal(mainWindow.shown, true)
  assert.equal(mainWindow.focused, true)
  assert.deepEqual(releases, [{ tabId: 'tab-a', senderId: detached.webContents.id }])
  assert.deepEqual(claims.at(-1), { tabId: 'tab-a', senderId: mainWindow.webContents.id })
})

test('closing the connection window does not claim the removed tab in main', () => {
  const { claims, detachedWindows, mainWindow, registry, releases } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)

  registry.closeTabWindow('tab-a')

  assert.equal(detached.closed, true)
  assert.deepEqual(releases, [{ tabId: 'tab-a', senderId: detached.webContents.id }])
  assert.equal(
    claims.some((claim) => claim.senderId === mainWindow.webContents.id),
    false
  )
})

test('claiming a replacement renderer immediately restores the terminal transcript', () => {
  const runtime = createRuntime()
  const sender = new FakeWebContents(2)
  runtime.set('tab-a', {
    profileId: 'profile-a',
    summary: 'SSH connected',
    terminalTranscript: 'login\r\nprompt$ command\r\nresult\r\n',
    remotePath: '/',
    remoteFiles: [],
    connected: true
  })

  runtime.claimTabRenderer('tab-a', sender)

  assert.deepEqual(sender.sent[0], {
    channel: 'terminal:state',
    payload: {
      tabId: 'tab-a',
      summary: 'SSH connected',
      transcript: 'login\r\nprompt$ command\r\nresult\r\n',
      connected: true
    }
  })
})

test('late cleanup of an old renderer preserves the replacement owner', () => {
  const runtime = createRuntime()
  const oldSender = new FakeWebContents(1)
  const currentSender = new FakeWebContents(2)

  runtime.claimTabRenderer('tab-a', oldSender)
  runtime.claimTabRenderer('tab-a', currentSender)
  runtime.releaseTabRenderer('tab-a', oldSender)
  oldSender.emit('destroyed')

  assert.equal(runtime.getTabRenderer('tab-a'), currentSender)
})
