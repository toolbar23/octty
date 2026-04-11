const COMPLETE_ESCAPE_PREFIX =
  /^(?:\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-_]|\x00|\x01|\x02|\x03|\x04|\x05|\x06|\x08|\x0b|\x0c|\x0e|\x0f|\x10|\x11|\x12|\x13|\x14|\x15|\x16|\x17|\x18|\x19|\x1a|\x1c|\x1d|\x1e|\x1f|\x7f)+/;

function trimLeadingControlSequences(value: string): string {
  let next = value;
  while (true) {
    const trimmed = next.replace(COMPLETE_ESCAPE_PREFIX, "");
    if (trimmed === next) {
      return trimmed;
    }
    next = trimmed;
  }
}

function maybeDropPartialFirstLine(value: string): string {
  const lineBreakIndex = value.search(/[\r\n]/);
  if (lineBreakIndex <= 0 || lineBreakIndex === value.length - 1) {
    return value;
  }

  const firstLine = value.slice(0, lineBreakIndex);
  const looksLikeClippedAnsiTail =
    /^[;?0-9:[\]()<>{} =]*[A-Za-z]?[;?0-9:[\]()<>{} =]*$/.test(firstLine) &&
    firstLine.includes("\x1b");

  if (!looksLikeClippedAnsiTail) {
    return value;
  }

  const dropLength =
    value[lineBreakIndex] === "\r" && value[lineBreakIndex + 1] === "\n"
      ? lineBreakIndex + 2
      : lineBreakIndex + 1;
  return value.slice(dropLength);
}

export function clipTerminalTranscript(value: string, maxChars: number): string {
  if (!value || maxChars <= 0) {
    return "";
  }

  let clipped = value;
  if (clipped.length > maxChars) {
    clipped = clipped.slice(-maxChars);
    const firstLineBreakIndex = clipped.search(/[\r\n]/);
    if (
      firstLineBreakIndex > 0 &&
      firstLineBreakIndex < Math.min(2048, Math.floor(maxChars / 8)) &&
      firstLineBreakIndex < clipped.length - 1
    ) {
      const dropLength =
        clipped[firstLineBreakIndex] === "\r" && clipped[firstLineBreakIndex + 1] === "\n"
          ? firstLineBreakIndex + 2
          : firstLineBreakIndex + 1;
      clipped = clipped.slice(dropLength);
    }
  }

  clipped = trimLeadingControlSequences(clipped);
  clipped = maybeDropPartialFirstLine(clipped);
  return trimLeadingControlSequences(clipped);
}
