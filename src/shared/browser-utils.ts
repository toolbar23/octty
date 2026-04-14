export type BrowserNavigationTarget =
  | { kind: "embed"; url: string }
  | { kind: "external"; url: string };

export interface BrowserSurfaceRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const SEARCH_URL = "https://duckduckgo.com/";

function hasScheme(value: string): boolean {
  return /^[a-z][a-z0-9+.-]*:/i.test(value);
}

function localHostLike(value: string): boolean {
  const host = value.split(/[/?#]/, 1)[0]?.toLowerCase() ?? "";
  return (
    host === "localhost" ||
    host.startsWith("localhost:") ||
    host === "127.0.0.1" ||
    host.startsWith("127.0.0.1:") ||
    host === "[::1]" ||
    host.startsWith("[::1]:")
  );
}

function hostLike(value: string): boolean {
  if (/\s/.test(value)) {
    return false;
  }
  const host = value.split(/[/?#]/, 1)[0] ?? "";
  if (localHostLike(value)) {
    return true;
  }
  return /^[a-z0-9-]+(\.[a-z0-9-]+)+(?::\d+)?$/i.test(host);
}

function normalizeUrl(url: URL): BrowserNavigationTarget {
  if (url.protocol === "http:" || url.protocol === "https:") {
    return { kind: "embed", url: url.href };
  }
  return { kind: "external", url: url.href };
}

export function normalizeBrowserNavigationInput(input: string): BrowserNavigationTarget {
  const trimmed = input.trim();
  if (!trimmed) {
    return { kind: "embed", url: "about:blank" };
  }

  if (localHostLike(trimmed)) {
    return normalizeUrl(new URL(`http://${trimmed}`));
  }

  if (hasScheme(trimmed)) {
    try {
      return normalizeUrl(new URL(trimmed));
    } catch {
      return {
        kind: "embed",
        url: `${SEARCH_URL}?q=${encodeURIComponent(trimmed)}`,
      };
    }
  }

  if (hostLike(trimmed)) {
    return normalizeUrl(new URL(`https://${trimmed}`));
  }

  return {
    kind: "embed",
    url: `${SEARCH_URL}?q=${encodeURIComponent(trimmed)}`,
  };
}

export function clampBrowserZoom(zoomFactor: number): number {
  if (!Number.isFinite(zoomFactor)) {
    return 1;
  }
  return Math.max(0.5, Math.min(3, Math.round(zoomFactor * 100) / 100));
}

export function rectIntersection(
  left: BrowserSurfaceRect,
  right: BrowserSurfaceRect,
): BrowserSurfaceRect {
  const x = Math.max(left.x, right.x);
  const y = Math.max(left.y, right.y);
  const maxX = Math.min(left.x + left.width, right.x + right.width);
  const maxY = Math.min(left.y + left.height, right.y + right.height);
  return {
    x,
    y,
    width: Math.max(0, maxX - x),
    height: Math.max(0, maxY - y),
  };
}

export function isRectMostlyContained(
  rect: BrowserSurfaceRect,
  container: BrowserSurfaceRect,
  minimumRatio = 0.98,
): boolean {
  if (rect.width <= 0 || rect.height <= 0 || container.width <= 0 || container.height <= 0) {
    return false;
  }

  const intersection = rectIntersection(rect, container);
  const rectArea = rect.width * rect.height;
  const intersectionArea = intersection.width * intersection.height;
  return intersectionArea / rectArea >= minimumRatio;
}

export function visibleRectIntersection(
  rect: BrowserSurfaceRect,
  containers: BrowserSurfaceRect[],
): BrowserSurfaceRect | null {
  let visible = rect;
  for (const container of containers) {
    visible = rectIntersection(visible, container);
    if (visible.width <= 0 || visible.height <= 0) {
      return null;
    }
  }
  return visible;
}
