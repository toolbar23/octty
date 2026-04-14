import { describe, expect, test } from "vitest";
import {
  clipboardHtmlToFilePaths,
  clipboardTextToFilePaths,
  fileUrlToPath,
  quoteTerminalPathForPaste,
  quoteTerminalPathsForPaste,
} from "./terminal-clipboard";

describe("terminal clipboard path helpers", () => {
  test("quotes paths only when the shell needs it", () => {
    expect(quoteTerminalPathForPaste("/tmp/screenshot.png")).toBe("/tmp/screenshot.png");
    expect(quoteTerminalPathForPaste("/tmp/screen shot.png")).toBe("'/tmp/screen shot.png'");
    expect(quoteTerminalPathForPaste("/tmp/it's.png")).toBe("'/tmp/it'\\''s.png'");
    expect(quoteTerminalPathsForPaste(["/tmp/a.png", "/tmp/b b.png"])).toBe(
      "/tmp/a.png '/tmp/b b.png'",
    );
  });

  test("extracts file URLs and absolute paths from clipboard text", () => {
    expect(clipboardTextToFilePaths("copy\nfile:///tmp/screen%20shot.png\n")).toEqual([
      "/tmp/screen shot.png",
    ]);
    expect(clipboardTextToFilePaths("/tmp/screen shot.png")).toEqual([
      "/tmp/screen shot.png",
    ]);
    expect(clipboardTextToFilePaths("not a filename")).toEqual([]);
  });

  test("converts file URLs across local and unc styles", () => {
    expect(fileUrlToPath("file:///tmp/screen%20shot.png")).toBe("/tmp/screen shot.png");
    expect(fileUrlToPath("file:///C:/Users/pm/image.png")).toBe("C:\\Users\\pm\\image.png");
    expect(fileUrlToPath("file://server/share/image.png")).toBe("//server/share/image.png");
  });

  test("extracts local image references from clipboard HTML", () => {
    expect(
      clipboardHtmlToFilePaths('<img src="file:///tmp/screen%20shot.png"><a href="https://x.test">x</a>'),
    ).toEqual(["/tmp/screen shot.png"]);
  });
});
