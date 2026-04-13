import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdtempSync } from "node:fs";
import {
  __testOnly,
  buildTerminalLaunch,
  createEmbeddedSessionCorrelationId,
} from "./embedded-sessions";

describe("embedded session providers", () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "octty-embedded-session-"));
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  test("builds a plain launch when there is no external session", () => {
    expect(buildTerminalLaunch("codex", null)).toEqual({
      argv: ["codex"],
      displayCommand: "codex",
    });
  });

  test("injects a correlation prompt into a fresh launch", () => {
    const correlationId = createEmbeddedSessionCorrelationId(123, "session-1");

    expect(buildTerminalLaunch("codex", null, correlationId)).toEqual({
      argv: ["codex", __testOnly.codexCorrelationPrompt(correlationId)],
      displayCommand: "codex",
    });
  });

  test("builds a resume launch when there is an external session", () => {
    expect(
      buildTerminalLaunch("codex", {
        provider: "codex",
        id: "session-ext",
        label: "Saved session",
        detectedAt: 1,
      }),
    ).toEqual({
      argv: ["codex", "resume", "session-ext"],
      displayCommand: "codex resume session-ext",
    });
  });

  test("detects a codex session by cwd and launch time", async () => {
    const launchedAt = Date.parse("2026-04-13T11:33:04.092Z");
    const dateDir = join(tempDir, "2026", "04", "13");
    mkdirSync(dateDir, { recursive: true });
    writeFileSync(
      join(dateDir, "rollout-2026-04-13T13-33-04-019d869d-d5ca-74a3-a1ba-b8a23a3b09d6.jsonl"),
      `${JSON.stringify({
        timestamp: "2026-04-13T11:33:17.235Z",
        type: "session_meta",
        payload: {
          id: "019d869d-d5ca-74a3-a1ba-b8a23a3b09d6",
          timestamp: "2026-04-13T11:33:04.092Z",
          cwd: "/home/pm/dev/workspac",
        },
      })}\n`,
    );

    const detected = await __testOnly.detectCodexSessionFromRoot({
      cwd: "/home/pm/dev/workspac",
      launchedAt,
      sessionsRoot: tempDir,
    });

    expect(detected).toMatchObject({
      provider: "codex",
      id: "019d869d-d5ca-74a3-a1ba-b8a23a3b09d6",
    });
  });

  test("detects a codex session by correlation id", async () => {
    const correlationId = createEmbeddedSessionCorrelationId(
      Date.parse("2026-04-13T11:33:04.092Z"),
      "session-1",
    );
    const dateDir = join(tempDir, "2026", "04", "13");
    mkdirSync(dateDir, { recursive: true });
    writeFileSync(
      join(dateDir, "rollout-2026-04-13T13-33-04-019d869d-d5ca-74a3-a1ba-b8a23a3b09d6.jsonl"),
      `${JSON.stringify({
        timestamp: "2026-04-13T11:33:17.235Z",
        type: "session_meta",
        payload: {
          id: "019d869d-d5ca-74a3-a1ba-b8a23a3b09d6",
          timestamp: "2026-04-13T11:33:04.092Z",
          cwd: "/home/pm/dev/workspac",
        },
      })}\n{"type":"response_item","payload":{"content":"${correlationId}"}}\n`,
    );
    writeFileSync(
      join(dateDir, "rollout-2026-04-13T13-33-09-019d869d-d5ca-74a3-a1ba-b8a23a3b09d7.jsonl"),
      `${JSON.stringify({
        timestamp: "2026-04-13T11:33:17.235Z",
        type: "session_meta",
        payload: {
          id: "019d869d-d5ca-74a3-a1ba-b8a23a3b09d7",
          timestamp: "2026-04-13T11:33:09.092Z",
          cwd: "/home/pm/dev/workspac",
        },
      })}\n`,
    );

    const detected = await __testOnly.detectCodexSessionFromRoot({
      cwd: "/home/pm/dev/workspac",
      correlationId,
      sessionsRoot: tempDir,
    });

    expect(detected).toMatchObject({
      provider: "codex",
      id: "019d869d-d5ca-74a3-a1ba-b8a23a3b09d6",
    });
  });

});
