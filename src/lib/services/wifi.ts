import type { WifiConfig_Deserialize, WifiSessionInfo } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

/**
 * Starts the single active Wi-Fi session and returns its connection details.
 *
 * The URL and token are returned only here: they are never persisted and cannot
 * be recovered from the job list after a page refresh. All binding, token
 * generation, staging and quota handling happen in Rust.
 */
export async function startWifiSession(
  config: WifiConfig_Deserialize,
): Promise<FetchResult<WifiSessionInfo>> {
  try {
    const info = await commands.startWifiSession(config);
    return { ok: true, value: info };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Wi-Fi session could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

/**
 * Requests an explicit stop, which publishes staged files to the destination.
 *
 * Two backend behaviours matter to callers:
 * - The command resolves only after the job has been finished, so a rejection
 *   usually means finalization already failed and the job is terminal.
 * - It also resolves successfully when no session is held (for example while a
 *   previous stop is still finalizing), so a resolved promise alone is not
 *   evidence that anything was published. Always confirm via the job status.
 *
 * The session token is never logged here.
 */
export async function stopWifiSession(): Promise<FetchResult<null>> {
  try {
    await commands.stopWifiSession();
    return { ok: true, value: null };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Wi-Fi session could not be stopped.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
