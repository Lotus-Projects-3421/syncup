type LogLevel = "info" | "warn" | "error";

function write(level: LogLevel, message: string, context?: Record<string, unknown>): void {
  const prefix = `[SyncUp ${new Date().toISOString()}]`;
  switch (level) {
    case "info":
      console.info(prefix, message, context ?? "");
      break;
    case "warn":
      console.warn(prefix, message, context ?? "");
      break;
    case "error":
      console.error(prefix, message, context ?? "");
      break;
  }
}

export const logger = {
  info(message: string, context?: Record<string, unknown>): void {
    write("info", message, context);
  },
  warn(message: string, context?: Record<string, unknown>): void {
    write("warn", message, context);
  },
  error(message: string, context?: Record<string, unknown>): void {
    write("error", message, context);
  },
};
