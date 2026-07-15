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

function createRegistry(initialTabIds = ['tab-a', 'tab-b'], failCloseTabId = null, registryOptions = {}) {
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
    isQuitting: () => false,
    ...registryOptions
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
const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))

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

test('workspace content drop appends to the actual sender window and ignores a spoofed target id', () => {
  const { detachedWindows, mainWindow, registry } = createRegistry(['tab-a', 'tab-b', 'tab-c'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.detach({ tabId: 'tab-b' })
  registry.claim('tab-b', detachedWindows[1].webContents)

  registry.startDrag({ dragId: 'drag-c', tabId: 'tab-c', sourceWindowId: 'main' }, mainWindow.webContents)
  registry.drop(
    {
      dragId: 'drag-c',
      tabId: 'tab-c',
      sourceWindowId: 'main',
      targetWindowId: 'detached-1',
      targetIndex: 1,
      dropZone: 'workspace'
    },
    detachedWindows[1].webContents
  )

  assert.deepEqual(registry.listPlacements(), [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session', order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 0 },
    { tabId: 'tab-c', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 1 }
  ])
})

test('exact tab-bar drop inserts at the requested target index', () => {
  const { detachedWindows, mainWindow, registry } = createRegistry(['tab-a', 'tab-b', 'tab-c'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  registry.startDrag({ dragId: 'drag-c', tabId: 'tab-c', sourceWindowId: 'main' }, mainWindow.webContents)
  registry.drop(
    {
      dragId: 'drag-c',
      tabId: 'tab-c',
      sourceWindowId: 'main',
      targetWindowId: 'detached-1',
      targetIndex: 0,
      dropZone: 'precise'
    },
    detachedWindows[0].webContents
  )

  assert.deepEqual(
    registry
      .listPlacements()
      .filter((placement) => placement.ownerWindowId === 'detached-1')
      .sort((left, right) => left.order - right.order)
      .map((placement) => placement.tabId),
    ['tab-c', 'tab-a', 'tab-b']
  )
})

test('same-window content drop is handled without reordering or detaching', async () => {
  const { detachedWindows, registry } = createRegistry(['tab-a', 'tab-b'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.move({ tabId: 'tab-b', targetWindowId: 'detached-1' })

  registry.startDrag({ dragId: 'drag-b', tabId: 'tab-b', sourceWindowId: 'detached-1' }, detachedWindows[0].webContents)
  registry.drop(
    {
      dragId: 'drag-b',
      tabId: 'tab-b',
      sourceWindowId: 'detached-1',
      targetWindowId: 'main',
      targetIndex: 0,
      dropZone: 'workspace'
    },
    detachedWindows[0].webContents
  )
  registry.finishDrag({ dragId: 'drag-b', detachIfUnhandled: true }, detachedWindows[0].webContents)
  await delay(100)

  assert.equal(detachedWindows.length, 1)
  assert.deepEqual(
    registry
      .listPlacements()
      .sort((left, right) => left.order - right.order)
      .map((placement) => placement.tabId),
    ['tab-a', 'tab-b']
  )
})

test('duplicate drop and late finish do not move or detach the tab twice', async () => {
  const { detachedWindows, mainWindow, registry } = createRegistry(['tab-a', 'tab-b'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)

  const input = { dragId: 'drag-b', tabId: 'tab-b', sourceWindowId: 'main' }
  registry.startDrag(input, mainWindow.webContents)
  registry.drop(
    { ...input, targetWindowId: 'detached-1', targetIndex: 1, dropZone: 'workspace' },
    detachedWindows[0].webContents
  )
  registry.drop(
    { ...input, targetWindowId: 'main', targetIndex: 0, dropZone: 'precise' },
    detachedWindows[0].webContents
  )
  registry.finishDrag({ dragId: input.dragId, detachIfUnhandled: true }, mainWindow.webContents)
  await delay(100)

  assert.equal(detachedWindows.length, 1)
  assert.deepEqual(
    registry
      .listPlacements()
      .filter((placement) => placement.ownerWindowId === 'detached-1')
      .sort((left, right) => left.order - right.order)
      .map((placement) => placement.tabId),
    ['tab-a', 'tab-b']
  )
})

test('closing and destroyed target renderers reject a workspace tab drop', () => {
  const closing = createRegistry(['tab-a', 'tab-b'])
  closing.registry.detach({ tabId: 'tab-a' })
  closing.registry.claim('tab-a', closing.detachedWindows[0].webContents)
  closing.registry.startDrag(
    { dragId: 'closing-target', tabId: 'tab-b', sourceWindowId: 'main' },
    closing.mainWindow.webContents
  )
  closing.detachedWindows[0].close()
  assert.throws(
    () =>
      closing.registry.drop(
        {
          dragId: 'closing-target',
          tabId: 'tab-b',
          sourceWindowId: 'main',
          targetWindowId: 'detached-1',
          dropZone: 'workspace'
        },
        closing.detachedWindows[0].webContents
      ),
    /Target workspace window is not available/
  )

  const destroyed = createRegistry(['tab-a', 'tab-b'])
  destroyed.registry.detach({ tabId: 'tab-a' })
  destroyed.registry.claim('tab-a', destroyed.detachedWindows[0].webContents)
  destroyed.registry.startDrag(
    { dragId: 'destroyed-target', tabId: 'tab-b', sourceWindowId: 'main' },
    destroyed.mainWindow.webContents
  )
  destroyed.detachedWindows[0].webContents.destroy()
  assert.throws(
    () =>
      destroyed.registry.drop(
        {
          dragId: 'destroyed-target',
          tabId: 'tab-b',
          sourceWindowId: 'main',
          targetWindowId: 'detached-1',
          dropZone: 'workspace'
        },
        destroyed.detachedWindows[0].webContents
      ),
    /Target workspace window is not available/
  )
})

test('unregistered standalone renderers cannot start or receive a workspace tab drag', () => {
  const { mainWindow, registry } = createRegistry(['tab-a'])
  const standaloneRenderer = new FakeWebContents(999)

  assert.throws(
    () =>
      registry.startDrag({ dragId: 'standalone-source', tabId: 'tab-a', sourceWindowId: 'main' }, standaloneRenderer),
    /source is not registered/
  )

  const input = { dragId: 'standalone-target', tabId: 'tab-a', sourceWindowId: 'main' }
  registry.startDrag(input, mainWindow.webContents)
  assert.throws(
    () => registry.drop({ ...input, targetWindowId: 'main', dropZone: 'workspace' }, standaloneRenderer),
    /Target workspace window is not available/
  )
})

test('a stale drag cannot copy a tab after its authoritative placement changes', () => {
  const { detachedWindows, mainWindow, registry } = createRegistry(['tab-a', 'tab-b', 'tab-c'])
  registry.detach({ tabId: 'tab-a' })
  registry.claim('tab-a', detachedWindows[0].webContents)
  registry.detach({ tabId: 'tab-b' })
  registry.claim('tab-b', detachedWindows[1].webContents)

  const input = { dragId: 'stale-c', tabId: 'tab-c', sourceWindowId: 'main' }
  registry.startDrag(input, mainWindow.webContents)
  registry.move({ tabId: 'tab-c', targetWindowId: 'detached-1' })
  registry.drop({ ...input, targetWindowId: 'detached-2', dropZone: 'workspace' }, detachedWindows[1].webContents)

  assert.equal(registry.listPlacements().find((placement) => placement.tabId === 'tab-c')?.ownerWindowId, 'detached-1')
})

test('expired drag state cannot detach a later release', async () => {
  const { detachedWindows, mainWindow, registry } = createRegistry(['tab-a'], null, {
    dragRecordTtlMs: 10
  })
  registry.startDrag({ dragId: 'expired-a', tabId: 'tab-a', sourceWindowId: 'main' }, mainWindow.webContents)
  await delay(30)
  registry.finishDrag({ dragId: 'expired-a', detachIfUnhandled: true }, mainWindow.webContents)
  await delay(100)

  assert.equal(detachedWindows.length, 0)
  assert.deepEqual(registry.listPlacements(), [{ tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 }])
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
