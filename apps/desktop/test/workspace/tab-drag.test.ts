import assert from 'node:assert/strict'
import test from 'node:test'
import { isTabDragReleasedOutsideWindow } from '../../src/renderer/features/layout/tab-drag.ts'

const windowBounds = { x: 100, y: 100, width: 1200, height: 800 }

test('leaves Chromium zeroed dragend coordinates to the document dragleave state', () => {
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 0, screenY: 0 }, windowBounds), false)
})

test('detects a release beyond the native window bounds', () => {
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 1400, screenY: 500 }, windowBounds), true)
  assert.equal(isTabDragReleasedOutsideWindow({ screenX: 600, screenY: 500 }, windowBounds), false)
})
