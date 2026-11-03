import type {
  ArchiveTemplate,
  ArchiveTemplateOption,
  Capabilities,
  Config,
  Job,
  Settings,
} from "../types";
import { validate } from "./model";

export const browserOnly = import.meta.env.MODE === "browser";
interface ExtensionPermissions {
  request(options: { origins: string[] }): Promise<boolean>;
  contains(options: { origins: string[] }): Promise<boolean>;
}
const extension = (
  globalThis as unknown as { chrome?: { permissions?: ExtensionPermissions } }
).chrome?.permissions;
export const extensionMode = browserOnly && !!extension;
let worker: Worker | undefined;
let counter = 0;
const pending = new Map<
  number,
  { resolve: (value: unknown) => void; reject: (error: Error) => void }
>();
function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!worker) {
    worker = new Worker(new URL("./worker.ts", import.meta.url), {
      type: "module",
      name: "Zimsmith local engine",
    });
    worker.onmessage = (event) => {
      const request = pending.get(event.data.id);
      if (!request) return;
      pending.delete(event.data.id);
      if (event.data.error) request.reject(new Error(event.data.error));
      else request.resolve(event.data.value);
    };
    worker.onerror = () => {
      for (const request of pending.values())
        request.reject(
          new Error(
            "The browser engine stopped. Reload this workspace to recover saved jobs.",
          ),
        );
      pending.clear();
      worker?.terminate();
      worker = undefined;
    };
  }
  return new Promise((resolve, reject) => {
    const id = ++counter;
    pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
    worker!.postMessage({ id, command, args });
  });
}
async function permit() {
  if (
    extension &&
    !(await extension.request({ origins: ["http://*/*", "https://*/*"] }))
  )
    throw new Error(
      "Website access was not granted. Allow access when starting a crawl to fetch pages and linked assets.",
    );
  void navigator.storage.persist?.();
}
export const browserApi = {
  archiveTemplates: () => call<ArchiveTemplateOption[]>("archive_templates"),
  previewArchiveTemplate: (template: ArchiveTemplate) =>
    call<string>("preview_archive_template", { template }),
  settings: () => call<Settings>("get_settings"),
  updateSettings: (settings: Settings) =>
    call<Settings>("update_settings", { settings }),
  capabilities: () => call<Capabilities>("capabilities"),
  list: () => call<Job[]>("list_jobs"),
  get: (id: string) => call<Job>("get_job", { id }),
  create: async (config: Config) => {
    validate(config);
    await permit();
    return call<Job>("create_job", { config });
  },
  pause: (id: string) => call<Job>("pause_job", { id }),
  resume: async (id: string) => {
    await permit();
    return call<Job>("resume_job", { id });
  },
  editRetry: async (id: string, config: Config) => {
    await permit();
    return call<Job>("edit_job", { id, config });
  },
  build: async (id: string, selected: string[]) => {
    await permit();
    return call<Job>("build_job", { id, selected });
  },
  rebuild: async (id: string, template: ArchiveTemplate) => {
    await permit();
    return call<Job>("rebuild_job", { id, template });
  },
  open: async (_id: string): Promise<void> => {
    throw new Error("Save the ZIM and open it with your installed ZIM viewer.");
  },
  export: async (id: string) => {
    const { file, name } = await call<{ file: File; name: string }>(
      "export_archive",
      { id },
    );
    const url = URL.createObjectURL(file),
      anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = name;
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 5 * 60 * 1000);
    return "Download started. Check your browser's Downloads folder.";
  },
};
