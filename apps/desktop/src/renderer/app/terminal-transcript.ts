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
