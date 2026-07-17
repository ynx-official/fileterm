import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveWorkspaceTabPlacements } from '../../src/main/services/windows/workspace-window-placement.ts'
import { findTabMovedToWindow } from '../../src/renderer/app/workspace-tab-placement.ts'

test('detects the tab that moved into a target window', () => {
  const previous = [
    { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main' as const, order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session' as const, order: 0 }
  ]
  const next = [
    { tabId: 'tab-a', ownerWindowId: 'detached-1', ownerKind: 'detached-session' as const, order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'main', ownerKind: 'main' as const, order: 0 }
  ]

  assert.equal(findTabMovedToWindow(previous, next, 'detached-1'), 'tab-a')
  assert.equal(findTabMovedToWindow(previous, next, 'main'), 'tab-b')
})

test('does not treat initial placement hydration as a completed move', () => {
  assert.equal(
    findTabMovedToWindow([], [{ tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 }], 'main'),
    null
  )
})

test('workspace placements keep ordered ordinary tabs in the main window', () => {
  assert.deepEqual(resolveWorkspaceTabPlacements(['tab-a', 'tab-b'], []), [
    { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 },
    { tabId: 'tab-b', ownerWindowId: 'main', ownerKind: 'main', order: 1 }
  ])
})

test('workspace placements preserve explicit main-window order', () => {
  assert.deepEqual(resolveWorkspaceTabPlacements(['tab-a', 'tab-b'], [], 'main', ['tab-b', 'tab-a']), [
    { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 1 },
    { tabId: 'tab-b', ownerWindowId: 'main', ownerKind: 'main', order: 0 }
  ])
})

test('workspace placements expose only ready detached windows as owners', () => {
  assert.deepEqual(
    resolveWorkspaceTabPlacements(
      ['tab-a', 'tab-b'],
      [
        { tabId: 'tab-a', ownerWindowId: 'detached-1', ready: false, order: 0 },
        { tabId: 'tab-b', ownerWindowId: 'detached-2', ready: true, order: 0 }
      ]
    ),
    [
      { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main', order: 0 },
      { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session', order: 0 }
    ]
  )
})
