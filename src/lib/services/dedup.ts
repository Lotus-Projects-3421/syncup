import type { DedupManifestSummary, DuplicateGroup, ScanConfig } from "$lib/ipc";
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

/**
 * Lists persisted move manifests, newest first.
 *
 * `moveDuplicates` returns a job id, while the manifest id is generated inside
 * the backend and never returned. Callers snapshot this list before a move and
 * compare afterwards to discover which manifest was created. `potentiallyUndoable`
 * is advisory only; the undo command re-validates every path itself.
 */
export async function fetchManifests(): Promise<FetchResult<DedupManifestSummary[]>> {
  try {
    const manifests = await commands.listDedupManifests();
    return { ok: true, value: manifests };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate manifests could not be listed.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

/**
 * Starts a move of every duplicate in the given scan session.
 *
 * The backend moves the complete set: there is no per-file selection. It
 * re-verifies each retained file and each source before moving, refuses to
 * overwrite an occupied destination, and records per-operation outcomes in a
 * crash-safe manifest. The returned id is a JOB id, not a manifest id.
 *
 * A successful move consumes the scan session, so the preview can no longer be
 * retrieved afterwards.
 */
export async function startMove(
  scanJobId: string,
  confirmed: boolean,
): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.moveDuplicates(scanJobId, confirmed);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate move could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
