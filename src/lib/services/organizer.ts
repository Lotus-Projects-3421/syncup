import type { OrganizerConfig_Deserialize } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

/**
 * Starts an organizer run and returns the job id. All ZIP streaming, metadata
 * parsing, date precedence, naming and verified copying happen in Rust; this
 * wrapper only forwards the configuration and normalises errors.
 */
export async function startOrganizerRun(
  config: OrganizerConfig_Deserialize,
): Promise<FetchResult<string>> {
  try {
    const jobId = await commands.startOrganizer(config);
    return { ok: true, value: jobId };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Organizer run could not be started.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
