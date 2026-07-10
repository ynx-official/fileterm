export const CLINK_AUTOSUGGEST_HELP_URL = 'https://chrisant996.github.io/clink/clink.html#gettingstarted_autosuggest'

const CLINK_AUTOSUGGEST_PROMPT_PATTERN = new RegExp(
  '\\u001b\\[[0-9;]*m\\u001b\\[7m\\u001b\\[\\d+CF2\\u001b\\[27m[-=]' +
    '\\u001b\\]8;[^\\u0007\\u001b]*;' +
    escapeRegExp(CLINK_AUTOSUGGEST_HELP_URL) +
    '(?:\\u0007|\\u001b\\\\)List Suggestions(?:\\u001b\\[\\d+;\\d+H|\\u001b\\[\\d+G)',
  'g'
)

export function isClinkAutosuggestHelpUrl(uri: string) {
  return uri.startsWith(CLINK_AUTOSUGGEST_HELP_URL)
}

export function stripClinkAutosuggestPrompt(value: string) {
  return value.replace(CLINK_AUTOSUGGEST_PROMPT_PATTERN, '')
}

export function trimHydratedTerminalChunk(currentTranscript: string, chunk: string): string {
  if (!currentTranscript || !chunk) {
    return chunk
  }
  if (currentTranscript.endsWith(chunk)) {
    return ''
  }

  const suffix = currentTranscript.slice(-Math.min(currentTranscript.length, chunk.length))
  const patternLength = chunk.length
  const separatorIndex = patternLength
  const sequenceLength = patternLength + 1 + suffix.length
  const prefixLengths = new Uint32Array(sequenceLength)
  const symbolAt = (index: number) => {
    if (index < patternLength) {
      return chunk.charCodeAt(index)
    }
    if (index === separatorIndex) {
      return -1
    }
    return suffix.charCodeAt(index - separatorIndex - 1)
  }

  for (let index = 1; index < sequenceLength; index += 1) {
    let candidateLength = prefixLengths[index - 1] ?? 0
    while (candidateLength > 0 && symbolAt(index) !== symbolAt(candidateLength)) {
      candidateLength = prefixLengths[candidateLength - 1] ?? 0
    }
    if (symbolAt(index) === symbolAt(candidateLength)) {
      candidateLength += 1
    }
    prefixLengths[index] = candidateLength
  }

  const overlapLength = prefixLengths[sequenceLength - 1] ?? 0
  // A one- or two-character match is common terminal output, not reliable
  // evidence that an IPC batch was already included in a hydrated snapshot.
  // Full chunks are handled above; partial overlap must contain enough context
  // (normally a complete prompt) before it is safe to trim.
  return overlapLength >= 8 ? chunk.slice(overlapLength) : chunk
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
