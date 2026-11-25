import type { DuplicateGroup, ScanConfig } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

/**
 * Starts a duplicate scan and returns its job id.
 *
 * All walking, size grouping, quick hashing, full BLAKE2b hashing and retention
 * ordering happen in Rust. The backend never reports scan progress, so the job
 * stays `pending` until it reaches a terminal state; callers must render an
 * indeterminate state rather than a percentage.
 */
export async function startScan(config: ScanConfig): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.scanDuplicates(config);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate scan could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

/**
 * Reads the preview groups produced by a completed scan.
 *
 * Results live in memory only. They are consumed by a successful move and lost
 * on application restart, in which case the backend reports a validation error
 * and the caller must ask the user to scan again.
 */
export async function fetchScanResults(jobId: string): Promise<FetchResult<DuplicateGroup[]>> {
  try {
    const groups = await commands.getScanResults(jobId);
    return { ok: true, value: groups };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate scan results could not be read.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
