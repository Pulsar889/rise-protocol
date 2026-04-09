export type LogLevel = "info" | "warn" | "error" | "debug";

function timestamp(): string {
  return new Date().toISOString();
}

function log(level: LogLevel, module: string, message: string, meta?: Record<string, unknown>): void {
  const entry: Record<string, unknown> = {
    ts: timestamp(),
    level,
    module,
    msg: message,
    ...meta,
  };
  const line = JSON.stringify(entry);
  if (level === "error") {
    console.error(line);
  } else {
    console.log(line);
  }
}

export function makeLogger(module: string) {
  return {
    info:  (msg: string, meta?: Record<string, unknown>) => log("info",  module, msg, meta),
    warn:  (msg: string, meta?: Record<string, unknown>) => log("warn",  module, msg, meta),
    error: (msg: string, meta?: Record<string, unknown>) => log("error", module, msg, meta),
    debug: (msg: string, meta?: Record<string, unknown>) => log("debug", module, msg, meta),
  };
}
