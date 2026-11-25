import type { BackupConfig, FolderNode, MtpDevice } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

/**
 * Enumerates MTP devices already mounted by GVFS. The backend does not trigger
 * a mount, so an empty list means the phone is not mounted yet.
 */
export async function discoverDevices(): Promise<FetchResult<MtpDevice[]>> {
  try {
    const devices = await commands.discoverDevices();
    return { ok: true, value: devices };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Device discovery failed.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

/**
 * Returns the device folder tree. Depth and entry count are bounded in Rust;
 * relPath is relative to the mount root and must be forwarded unchanged.
 */
export async function browseFolders(deviceId: string): Promise<FetchResult<FolderNode[]>> {
  try {
    const folders = await commands.browseFolders(deviceId);
    return { ok: true, value: folders };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Folder browsing failed.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

/**
 * Starts a backup job and returns its id. All walking, copying, verification
 * and atomic publication happen in Rust.
 */
export async function startBackup(config: BackupConfig): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.startBackup(config);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Backup could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
