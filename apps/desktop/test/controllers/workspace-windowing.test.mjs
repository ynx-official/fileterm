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

function createRegistry(initialTabIds = ['tab-a', 'tab-b'], failCloseTabId = null) {
  const mainWindow = new FakeBrowserWindow(1)
  const detachedWindows = []
  const claims = []
  const releases = []
  const closeCalls = []
  let tabIds = [...initialTabIds]
  const registry = new WorkspaceWindowRegistry({
    getMainWindow: () => mainWindow,
    listTabIds: () => tabIds,
    createDetachedWindow: () => {
      const window = new FakeBrowserWindow(100 + detachedWindows.length)
      detachedWindows.push(window)
      return window
    },
    claimTabRenderer: (tabId, sender) => claims.push({ tabId, senderId: sender.id }),
    releaseTabRenderer: (tabId, sender) => releases.push({ tabId, senderId: sender.id }),
    closeTab: async (tabId) => {
      if (tabId === failCloseTabId) {
        throw new Error(`close failed: ${tabId}`)
      }
      closeCalls.push(tabId)
      tabIds = tabIds.filter((entry) => entry !== tabId)
    },
    broadcastPlacements: () => undefined,
    isQuitting: () => false
  })
  registry.registerMainWindow(mainWindow)
  return { claims, closeCalls, detachedWindows, mainWindow, registry, releases }
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

const nextTurn = () => new Promise((resolve) => setImmediate(resolve))

test('detached tab ownership moves only after its renderer claims the tab', () => {
  const { claims, detachedWindows, registry } = createRegistry(['tab-a'])
  registry.detach({ tabId: 'tab-a' })
  assert.deepEqual(registry.listPlacements(), [{ tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 }])

  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)

  assert.deepEqual(claims, [{ tabId: 'tab-a', senderId: detached.webContents.id }])
  assert.equal(detached.shown, true)
  assert.equal(detached.focused, true)
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 }
  ])
})

test('a detached workspace window accepts multiple ordered tabs', () => {
  const { claims, detachedWindows, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)

  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1', targetIndex: 0 })

  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 1 },
    { tabId: 'tab-b', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 }
  ])
  assert.deepEqual(claims.at(-1), { tabId: 'tab-b', senderId: detached.webContents.id })
})

test('moving one grouped tab to a new window preserves the other tabs', () => {
  const { detachedWindows, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  registry.detach({ tabId: 'tab-b' })
  assert.equal(detachedWindows.length, 2)
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 1 }
  ])

  registry.claim('tab-b', detachedWindows[1].webContents)

  assert.equal(detachedWindows[0].closed, false)
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 0 }
  ])
})

test('tabs move between detached windows and the empty source window closes', () => {
  const { detachedWindows, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.detach({ tabId: 'tab-b' })
  registry.claim('tab-b', detachedWindows[1].webContents)

  registry.move({ tabId: 'tab-a', targetWindowId: 'detached-2', targetIndex: 1 })

  assert.equal(detachedWindows[0].closed, true)
  assert.equal(detachedWindows[1].closed, false)
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 1 },
    { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 0 }
  ])
})

test('moving the final detached tab to main closes only the empty window', () => {
  const { claims, detachedWindows, mainWindow, registry } = createRegistry(['tab-a'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  mainWindow.minimized = true

  registry.attach('tab-a')

  assert.equal(detachedWindows[0].closed, true)
  assert.equal(mainWindow.restored, true)
  assert.equal(mainWindow.focused, true)
  assert.deepEqual(claims.at(-1), { tabId: 'tab-a', senderId: mainWindow.webContents.id })
  assert.deepEqual(registry.listPlacements(), [{ tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 }])
})

test('closing a grouped detached window closes every contained connection', async () => {
  const { claims, closeCalls, detachedWindows, mainWindow, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  detached.close()
  await nextTurn()

  assert.equal(detached.closed, true)
  assert.deepEqual(closeCalls, ['tab-a', 'tab-b'])
  assert.equal(
    claims.some((claim) => claim.senderId === mainWindow.webContents.id),
    false
  )
})

test('a grouped window stays open with remaining tabs when one connection fails to close', async () => {
  const { closeCalls, detachedWindows, registry } = createRegistry(['tab-a', 'tab-b'], 'tab-b')
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  detached.close()
  await nextTurn()

  assert.equal(detached.closed, false)
  assert.deepEqual(closeCalls, ['tab-a'])
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-b', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 }
  ])
})

test('renderer failure restores every grouped tab to main without closing connections', () => {
  const { claims, closeCalls, detachedWindows, mainWindow, registry } = createRegistry()
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  detached.webContents.emit('render-process-gone')

  assert.deepEqual(closeCalls, [])
  assert.deepEqual(
    claims.filter((claim) => claim.senderId === mainWindow.webContents.id),
    [
      { tabId: 'tab-a', senderId: mainWindow.webContents.id },
      { tabId: 'tab-b', senderId: mainWindow.webContents.id }
    ]
  )
  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'main', ownerKind: 'main', order: 1 }
  ])
})

test('closing the last connection tab destroys its detached window without claiming main', () => {
  const { claims, detachedWindows, mainWindow, registry } = createRegistry(['tab-a'])
  registry.detach({ tabId: 'tab-a' })
  const detached = detachedWindows[0]
  registry.claim('tab-a', detached.webContents)

  registry.closeTabWindow('tab-a')

  assert.equal(detached.closed, true)
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
