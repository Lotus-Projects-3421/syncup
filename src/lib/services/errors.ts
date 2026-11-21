export interface AppError {
  code: string;
  message: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Normalises values rejected by Tauri IPC into the structured shape used by the UI.
 * Rust never exposes internal paths or stack traces; this only preserves the
 * `code`/`message` contract produced by `IpcError`.
 */
export function toAppError(error: unknown): AppError {
  if (
    isRecord(error) &&
    typeof error.code === "string" &&
    typeof error.message === "string"
  ) {
    return { code: error.code, message: error.message };
  }

  if (error instanceof Error) {
    return { code: "UNKNOWN", message: error.message };
  }

  if (typeof error === "string") {
    return { code: "UNKNOWN", message: error };
  }

  return { code: "UNKNOWN", message: "An unexpected error occurred." };
}
