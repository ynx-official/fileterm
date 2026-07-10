import assert from 'node:assert/strict'
import test from 'node:test'
import { trimHydratedTerminalChunk } from '../../src/renderer/app/terminal-transcript.ts'

test('removes a fully hydrated terminal chunk', () => {
  assert.equal(trimHydratedTerminalChunk('banner\r\nprompt$ ', 'prompt$ '), '')
})

test('keeps only the non-hydrated tail of a partially overlapping chunk', () => {
  assert.equal(trimHydratedTerminalChunk('banner\r\nprompt$ ', 'prompt$ command\r\n'), 'command\r\n')
})

test('preserves chunks that do not overlap hydrated output', () => {
  assert.equal(trimHydratedTerminalChunk('prompt$ ', '\u001b[32mresult\u001b[0m\r\n'), '\u001b[32mresult\u001b[0m\r\n')
})

test('does not trim a coincidental short overlap from real output', () => {
  assert.equal(trimHydratedTerminalChunk('job: ok\nabc', 'abcdef\n'), 'abcdef\n')
})

test('handles ANSI and CRLF overlap split across IPC batches', () => {
  const current = 'login\r\n\u001b[32mstoffel@debian\u001b[0m:~$ '
  const chunk = '\u001b[32mstoffel@debian\u001b[0m:~$ printf ok\r\nok\r\n'
  assert.equal(trimHydratedTerminalChunk(current, chunk), 'printf ok\r\nok\r\n')
})
