import type { DedupManifestSummary, DuplicateGroup, ScanConfig } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

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

export async function startMove(scanJobId: string, confirmed: boolean): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.moveDuplicates(scanJobId, confirmed);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate move could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

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
 * Starts an undo job to restore files from quarantine to their original locations.
 *
 * The backend re-validates every path: original locations must be unoccupied,
 * quarantine files must match their recorded hashes, and occupied destinations
 * are marked UndoBlocked rather than overwritten. Operations already marked
 * Undone are skipped. The returned id is a job id for progress tracking.
 */
export async function startUndo(manifestId: string): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.undoDuplicates(manifestId);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Duplicate undo could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
