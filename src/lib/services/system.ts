import type { HealthStatus } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError, type AppError } from "./errors";
import { logger } from "./logger";

export type FetchResult<T> = { ok: true; value: T } | { ok: false; error: AppError };

export async function fetchHealth(): Promise<FetchResult<HealthStatus>> {
  try {
    const health = await commands.healthCheck();
    return { ok: true, value: health };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Health check failed.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
