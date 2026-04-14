import { app, clipboard, type NativeImage } from "electron";
import { randomUUID } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  clipboardHtmlToFilePaths,
  clipboardTextToFilePaths,
  quoteTerminalPathForPaste,
  terminalClipboardPasteFromPaths,
  type TerminalClipboardPaste,
} from "../shared/terminal-clipboard";

const clipboardTextFormats = [
  "text/uri-list",
  "x-special/gnome-copied-files",
  "public.file-url",
  "public.url",
];

const clipboardFileBufferFormats = ["FileNameW", "FileName"];

export async function readTerminalClipboardPaste(): Promise<TerminalClipboardPaste> {
  const paths = readClipboardFilePaths();
  if (paths.length > 0) {
    return terminalClipboardPasteFromPaths(paths);
  }

  const image = clipboard.readImage();
  if (!image.isEmpty()) {
    const filePath = await writeClipboardImageToTempFile(image);
    return {
      source: "image",
      text: quoteTerminalPathForPaste(filePath),
    };
  }

  const text = clipboard.readText();
  return {
    source: text ? "text" : "empty",
    text,
  };
}

function readClipboardFilePaths(): string[] {
  const paths: string[] = [];
  const formats = new Set(clipboard.availableFormats());

  for (const format of clipboardTextFormats) {
    const text = readClipboardFormatText(format, formats);
    paths.push(...clipboardTextToFilePaths(text));
  }

  paths.push(...clipboardTextToFilePaths(clipboard.readText()));
  paths.push(...clipboardHtmlToFilePaths(clipboard.readHTML()));

  try {
    const bookmark = clipboard.readBookmark();
    paths.push(...clipboardTextToFilePaths(bookmark.url));
  } catch {
    // readBookmark is platform-specific.
  }

  for (const format of clipboardFileBufferFormats) {
    if (!formats.has(format)) {
      continue;
    }
    paths.push(...readClipboardBufferPaths(format));
  }

  return Array.from(new Set(paths));
}

function readClipboardFormatText(format: string, formats: Set<string>): string {
  if (!formats.has(format)) {
    return "";
  }

  try {
    const text = clipboard.read(format);
    if (text) {
      return text;
    }
  } catch {
    // Some platforms advertise formats Electron cannot expose as text.
  }

  try {
    return clipboard.readBuffer(format).toString("utf8");
  } catch {
    return "";
  }
}

function readClipboardBufferPaths(format: string): string[] {
  try {
    const buffer = clipboard.readBuffer(format);
    const text = format === "FileNameW" ? decodeNullDelimitedUtf16(buffer) : decodeNullDelimitedUtf8(buffer);
    return clipboardTextToFilePaths(text);
  } catch {
    return [];
  }
}

function decodeNullDelimitedUtf16(buffer: Buffer): string {
  return buffer.toString("utf16le").replace(/\0+$/g, "").split("\0").join("\n");
}

function decodeNullDelimitedUtf8(buffer: Buffer): string {
  return buffer.toString("utf8").replace(/\0+$/g, "").split("\0").join("\n");
}

async function writeClipboardImageToTempFile(image: NativeImage): Promise<string> {
  const directory = join(app.getPath("temp"), "octty-clipboard");
  await mkdir(directory, { recursive: true });
  const filePath = join(directory, `clipboard-image-${Date.now()}-${randomUUID()}.png`);
  await writeFile(filePath, image.toPNG());
  return filePath;
}
