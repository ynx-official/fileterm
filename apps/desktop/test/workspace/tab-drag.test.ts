import assert from 'node:assert/strict'
import test from 'node:test'
import {
  isTabDragReleasedOutsideWindow,
  isWorkspaceTabDrag,
  resolveWorkspaceTabDropTargetIndex,
  WORKSPACE_TAB_DRAG_MIME
} from '../../src/renderer/features/layout/tab-drag.ts'

const windowBounds = { x: 100, y: 100, width: 1200, height: 800 }

test('leaves Chromium zeroed dragend coordinates to the document dragleave state', () => {
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 0, screenY: 0 }, windowBounds), false)
})

test('detects a release beyond the native window bounds', () => {
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 1400, screenY: 500 }, windowBounds), true)
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 600, screenY: 500 }, windowBounds), false)
})

test('accepts only the internal workspace tab MIME', () => {
  assert.equal(isWorkspaceTabDrag({ types: [WORKSPACE_TAB_DRAG_MIME, 'text/plain'] }), true)
  assert.equal(isWorkspaceTabDrag({ types: ['Files', 'text/plain'] }), false)
  assert.equal(isWorkspaceTabDrag(null), false)
})

test('appends a cross-window fallback drop to the target session list', () => {
  assert.equal(
    resolveWorkspaceTabDropTargetIndex({
      sessionTabIds: ['tab-a', 'tab-b'],
      draggedTabId: 'tab-c',
      isSameWindow: false
    }),
    2
  )
})

test('preserves order for a same-window content fallback drop', () => {
  assert.equal(
    resolveWorkspaceTabDropTargetIndex({
      sessionTabIds: ['tab-a', 'tab-b', 'tab-c'],
      draggedTabId: 'tab-b',
      isSameWindow: true,
      preserveCurrentOrder: true
    }),
    1
  )
})

test('adjusts an exact insertion index after removing the source tab', () => {
  assert.equal(
    resolveWorkspaceTabDropTargetIndex({
      sessionTabIds: ['tab-a', 'tab-b', 'tab-c'],
      draggedTabId: 'tab-a',
      isSameWindow: true,
      targetTabId: 'tab-c'
    }),
    1
  )
})
