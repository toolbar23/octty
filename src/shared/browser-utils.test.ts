import { describe, expect, test } from "vitest";
import {
  clampBrowserZoom,
  isRectMostlyContained,
  normalizeBrowserNavigationInput,
  rectIntersection,
  visibleRectIntersection,
} from "./browser-utils";

describe("browser utils", () => {
  test("normalizes embeddable URLs and host-like input", () => {
    expect(normalizeBrowserNavigationInput("https://example.com/docs")).toEqual({
      kind: "embed",
      url: "https://example.com/docs",
    });
    expect(normalizeBrowserNavigationInput("example.com/docs")).toEqual({
      kind: "embed",
      url: "https://example.com/docs",
    });
    expect(normalizeBrowserNavigationInput("localhost:5173")).toEqual({
      kind: "embed",
      url: "http://localhost:5173/",
    });
    expect(normalizeBrowserNavigationInput("127.0.0.1:8080/app")).toEqual({
      kind: "embed",
      url: "http://127.0.0.1:8080/app",
    });
  });

  test("turns plain text into a search URL and external protocols into external targets", () => {
    expect(normalizeBrowserNavigationInput("jj bookmarks")).toEqual({
      kind: "embed",
      url: "https://duckduckgo.com/?q=jj%20bookmarks",
    });
    expect(normalizeBrowserNavigationInput("mailto:test@example.com")).toEqual({
      kind: "external",
      url: "mailto:test@example.com",
    });
  });

  test("clamps zoom to Chromium-like browser bounds", () => {
    expect(clampBrowserZoom(Number.NaN)).toBe(1);
    expect(clampBrowserZoom(0.2)).toBe(0.5);
    expect(clampBrowserZoom(1.234)).toBe(1.23);
    expect(clampBrowserZoom(4)).toBe(3);
  });

  test("computes surface containment", () => {
    const viewport = { x: 0, y: 0, width: 100, height: 100 };

    expect(rectIntersection({ x: 90, y: 10, width: 20, height: 20 }, viewport)).toEqual({
      x: 90,
      y: 10,
      width: 10,
      height: 20,
    });
    expect(isRectMostlyContained({ x: 5, y: 5, width: 90, height: 90 }, viewport)).toBe(true);
    expect(isRectMostlyContained({ x: 90, y: 5, width: 90, height: 90 }, viewport)).toBe(false);
  });

  test("clips browser surfaces to visible container intersections", () => {
    expect(
      visibleRectIntersection(
        { x: 80, y: 10, width: 50, height: 40 },
        [{ x: 0, y: 0, width: 100, height: 100 }],
      ),
    ).toEqual({ x: 80, y: 10, width: 20, height: 40 });
    expect(
      visibleRectIntersection(
        { x: 110, y: 10, width: 50, height: 40 },
        [{ x: 0, y: 0, width: 100, height: 100 }],
      ),
    ).toBeNull();
  });
});
