import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveWorkspaceTabPlacements } from '../../src/main/services/windows/workspace-window-placement.ts'

test('workspace placements keep ordinary tabs in the main window', () => {
  assert.deepEqual(resolveWorkspaceTabPlacements(['tab-a', 'tab-b'], []), [
    { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main' },
    { tabId: 'tab-b', ownerWindowId: 'main', ownerKind: 'main' }
  ])
})

test('workspace placements expose only ready detached windows as owners', () => {
  assert.deepEqual(
    resolveWorkspaceTabPlacements(
      ['tab-a', 'tab-b'],
      [
        { tabId: 'tab-a', ownerWindowId: 'detached-1', ready: false },
        { tabId: 'tab-b', ownerWindowId: 'detached-2', ready: true }
      ]
    ),
    [
      { tabId: 'tab-a', ownerWindowId: 'main', ownerKind: 'main' },
      { tabId: 'tab-b', ownerWindowId: 'detached-2', ownerKind: 'detached-session' }
    ]
  )
})
