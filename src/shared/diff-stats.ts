export interface DiffStats {
  addedLines: number;
  removedLines: number;
}

export function summarizeUnifiedDiff(diffText: string): DiffStats {
  let addedLines = 0;
  let removedLines = 0;

  for (const line of diffText.split("\n")) {
    if (line.startsWith("+++")) {
      continue;
    }
    if (line.startsWith("---")) {
      continue;
    }
    if (line.startsWith("+")) {
      addedLines += 1;
      continue;
    }
    if (line.startsWith("-")) {
      removedLines += 1;
    }
  }

  return { addedLines, removedLines };
}
