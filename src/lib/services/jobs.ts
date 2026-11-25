import type { Job_Serialize } from "$lib/ipc";
import { commands } from "$lib/ipc";
import { toAppError } from "./errors";
import { logger } from "./logger";
import type { FetchResult } from "./system";

export async function fetchJobs(): Promise<FetchResult<Job_Serialize[]>> {
  try {
    const jobs = await commands.listJobs();
    return { ok: true, value: jobs };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Job list could not be loaded.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

export async function fetchJob(jobId: string): Promise<FetchResult<Job_Serialize>> {
  try {
    const job = await commands.getJob(jobId);
    return { ok: true, value: job };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Job status could not be read.", { code: appError.code });
    return { ok: false, error: appError };
  }
}

export async function cancelJob(jobId: string): Promise<FetchResult<null>> {
  try {
    await commands.cancelJob(jobId);
    return { ok: true, value: null };
  } catch (error) {
    const appError = toAppError(error);
    logger.error("Job could not be cancelled.", { code: appError.code });
    return { ok: false, error: appError };
  }
}
