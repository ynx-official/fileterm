import assert from 'node:assert/strict'
import test from 'node:test'
import {
  canDetachWorkspaceTabFromWindow,
  isTabDragReleasedOutsideWindow,
  isWorkspaceTabDrag,
  resolveWorkspaceTabDropTargetIndex,
  resolveWorkspaceTabOutsideFeedback,
  FILETERM_TAB_DRAG_MIME,
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

test('recognizes either trusted FileTerm tab MIME without claiming external drags', () => {
  assert.equal(isWorkspaceTabDrag({ types: [WORKSPACE_TAB_DRAG_MIME, 'text/plain'] }), true)
  assert.equal(isWorkspaceTabDrag({ types: [FILETERM_TAB_DRAG_MIME] }), true)
  assert.equal(isWorkspaceTabDrag({ types: ['Files', 'text/plain'] }), false)
  assert.equal(isWorkspaceTabDrag({ types: ['text/plain'] }), false)
  assert.equal(isWorkspaceTabDrag({ types: ['application/x-fileterm-local-tab'] }), false)
  assert.equal(isWorkspaceTabDrag(null), false)
})

test('allows unhandled detach only from main or a multi-session detached window', () => {
  assert.equal(canDetachWorkspaceTabFromWindow('main', 1), true)
  assert.equal(canDetachWorkspaceTabFromWindow('detached-session', 2), true)
  assert.equal(canDetachWorkspaceTabFromWindow('detached-session', 1), false)
})

test('uses merge feedback for a single-tab detached window instead of blocking the drag', () => {
  assert.equal(resolveWorkspaceTabOutsideFeedback(true, false), 'attach')
  assert.equal(resolveWorkspaceTabOutsideFeedback(true, true), 'detach')
  assert.equal(resolveWorkspaceTabOutsideFeedback(false, false), 'blocked')
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
