import type { TerminalKind } from "./types";

const TERMINAL_KIND_LABELS: Record<TerminalKind, string> = {
  shell: "Shell",
  codex: "Codex",
  pi: "Pi",
  nvim: "Nvim",
  jjui: "Jjui",
};

export function normalizeTerminalKind(kind: string | null | undefined): TerminalKind {
  switch (kind) {
    case "codex":
    case "pi":
    case "nvim":
    case "jjui":
    case "shell":
      return kind;
    case "agent-shell":
      return "codex";
    default:
      return "shell";
  }
}

export function terminalKindLabel(kind: TerminalKind): string {
  return TERMINAL_KIND_LABELS[kind];
}

export function defaultTerminalCommand(kind: TerminalKind): string {
  switch (kind) {
    case "shell":
      return "shell";
    case "codex":
      return "codex";
    case "pi":
      return "pi";
    case "nvim":
      return "nvim";
    case "jjui":
      return "jjui";
  }
}

export function isAgentTerminalKind(kind: TerminalKind): boolean {
  return kind === "codex" || kind === "pi";
}
