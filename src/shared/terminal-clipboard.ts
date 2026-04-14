export type TerminalClipboardPasteSource = "empty" | "file" | "image" | "text";

export interface TerminalClipboardPaste {
  text: string;
  source: TerminalClipboardPasteSource;
}

const clipboardPathIgnoredLines = new Set(["copy", "cut"]);
const safeShellPathPattern = /^[A-Za-z0-9_./:@%+=,-]+$/;

export function quoteTerminalPathForPaste(path: string): string {
  if (safeShellPathPattern.test(path)) {
    return path;
  }

  return `'${path.replaceAll("'", "'\\''")}'`;
}

export function quoteTerminalPathsForPaste(paths: string[]): string {
  return paths.map(quoteTerminalPathForPaste).join(" ");
}

export function clipboardTextToFilePaths(text: string): string[] {
  const candidates = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"))
    .filter((line) => !clipboardPathIgnoredLines.has(line.toLowerCase()));

  if (candidates.length === 0) {
    return [];
  }

  const paths: string[] = [];
  for (const candidate of candidates) {
    const fileUrlPath = fileUrlToPath(candidate);
    if (fileUrlPath) {
      paths.push(fileUrlPath);
      continue;
    }

    if (!isLikelyAbsolutePath(candidate)) {
      return [];
    }
    paths.push(candidate);
  }

  return paths;
}

export function clipboardHtmlToFilePaths(html: string): string[] {
  const paths: string[] = [];
  const attributePattern = /\b(?:src|href)\s*=\s*(["'])(file:\/\/.+?)\1/giu;
  for (const match of html.matchAll(attributePattern)) {
    const path = fileUrlToPath(match[2] ?? "");
    if (path) {
      paths.push(path);
    }
  }
  return unique(paths);
}

export function terminalClipboardPasteFromPaths(paths: string[]): TerminalClipboardPaste {
  if (paths.length === 0) {
    return { source: "empty", text: "" };
  }

  return {
    source: "file",
    text: quoteTerminalPathsForPaste(paths),
  };
}

export function fileUrlToPath(value: string): string | null {
  if (!/^file:\/\//i.test(value)) {
    return null;
  }

  try {
    const url = new URL(value);
    if (url.protocol !== "file:") {
      return null;
    }

    let pathname = decodeURIComponent(url.pathname);
    if (/^\/[A-Za-z]:\//.test(pathname)) {
      pathname = pathname.slice(1).replaceAll("/", "\\");
    } else if (url.hostname && url.hostname !== "localhost") {
      pathname = `//${url.hostname}${pathname}`;
    }

    return pathname;
  } catch {
    return null;
  }
}

function isLikelyAbsolutePath(value: string): boolean {
  return (
    value.startsWith("/") ||
    value.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(value)
  );
}

function unique(values: string[]): string[] {
  return Array.from(new Set(values));
}
