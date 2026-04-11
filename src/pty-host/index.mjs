import readline from "node:readline";
import pty from "node-pty";
import { createOutputBatcher } from "./output-batching.mjs";

const sessions = new Map();

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

const outputBatcher = createOutputBatcher({ send });

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

send({ type: "ready" });

rl.on("line", (line) => {
  if (!line.trim()) {
    return;
  }

  const message = JSON.parse(line);
  if (message.type === "create") {
    const proc = pty.spawn(message.command, message.args ?? [], {
      name: "xterm-256color",
      cols: message.cols ?? 120,
      rows: message.rows ?? 32,
      cwd: message.cwd || process.cwd(),
      env: process.env,
    });

    sessions.set(message.sessionId, proc);
    proc.onData((data) => {
      outputBatcher.add(message.sessionId, data);
    });
    proc.onExit((event) => {
      outputBatcher.flush(message.sessionId);
      send({
        type: "exit",
        sessionId: message.sessionId,
        exitCode: event.exitCode,
      });
      sessions.delete(message.sessionId);
    });
    return;
  }

  const proc = sessions.get(message.sessionId);
  if (!proc) {
    send({
      type: "error",
      sessionId: message.sessionId,
      message: "Session not found",
    });
    return;
  }

  if (message.type === "write") {
    proc.write(message.data ?? "");
    return;
  }

  if (message.type === "resize") {
    proc.resize(message.cols ?? 120, message.rows ?? 32);
    return;
  }

  if (message.type === "kill") {
    outputBatcher.flush(message.sessionId);
    proc.kill();
    sessions.delete(message.sessionId);
  }
});
