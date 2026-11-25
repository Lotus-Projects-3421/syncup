import type { AppSettings, MigrationReport, MigrationStatus_Serialize } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

export async function fetchSettings(): Promise<FetchResult<AppSettings>> {
  try {
    const settings = await commands.getSettings();
    return { ok: true, value: settings };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Settings could not be loaded.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

export async function saveSettings(settings: AppSettings): Promise<FetchResult<null>> {
  try {
    await commands.updateSettings(settings);
    return { ok: true, value: null };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Settings could not be saved.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

export async function fetchMigrationStatus(): Promise<FetchResult<MigrationStatus_Serialize>> {
  try {
    const status = await commands.getMigrationStatus();
    return { ok: true, value: status };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Migration status could not be read.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

export async function runMigration(
  sourceDbPath: string | null,
  sourceJsonDir: string | null,
): Promise<FetchResult<MigrationReport>> {
  try {
    const report = await commands.migrateLegacyImport(sourceDbPath, sourceJsonDir);
    return { ok: true, value: report };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Legacy import failed.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
