import type { BrowserJob } from "./model";

export interface SyncFile {
  write(data: ArrayBufferView, options?: { at?: number }): number;
  read(data: ArrayBufferView, options?: { at?: number }): number;
  getSize(): number;
  truncate(size: number): void;
  flush(): void;
  close(): void;
}
export async function syncFile(
  dir: FileSystemDirectoryHandle,
  name: string,
): Promise<SyncFile> {
  const file = await dir.getFileHandle(name, { create: true });
  return (
    file as FileSystemFileHandle & {
      createSyncAccessHandle(): Promise<SyncFile>;
    }
  ).createSyncAccessHandle();
}
export async function writeFile(
  dir: FileSystemDirectoryHandle,
  name: string,
  data: string | Uint8Array,
) {
  const bytes =
    typeof data === "string" ? new TextEncoder().encode(data) : data;
  const file = await syncFile(dir, name);
  try {
    file.truncate(0);
    if (file.write(bytes, { at: 0 }) !== bytes.byteLength)
      throw new Error("Browser storage returned a short write.");
    file.flush();
  } finally {
    file.close();
  }
}
export async function readFile(
  dir: FileSystemDirectoryHandle,
  name: string,
): Promise<File> {
  return (await dir.getFileHandle(name)).getFile();
}
export class Store {
  private constructor(
    private db: IDBDatabase,
    readonly root: FileSystemDirectoryHandle,
  ) {}
  static async open() {
    if (!navigator.storage?.getDirectory || !navigator.locks)
      throw new Error(
        "Use a current browser with local file storage and Web Locks, over HTTPS or localhost.",
      );
    const root = await (
      await navigator.storage.getDirectory()
    ).getDirectoryHandle("zimsmith", { create: true });
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("zimsmith-browser-v1", 1);
      request.onupgradeneeded = () => {
        for (const name of ["jobs", "preferences", "hosts"])
          request.result.createObjectStore(name);
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    return new Store(db, root);
  }
  async get<T>(table: string, key: string): Promise<T | undefined> {
    return new Promise((resolve, reject) => {
      const request = this.db.transaction(table).objectStore(table).get(key);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }
  async allJobs(): Promise<BrowserJob[]> {
    return new Promise((resolve, reject) => {
      const request = this.db.transaction("jobs").objectStore("jobs").getAll();
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  }
  async put(table: string, key: string, value: unknown) {
    // IndexedDB commits manifests atomically; a killed tab cannot leave half JSON.
    return new Promise<void>((resolve, reject) => {
      const tx = this.db.transaction(table, "readwrite");
      tx.objectStore(table).put(value, key);
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () =>
        reject(tx.error || new Error("Unable to save browser checkpoint."));
    });
  }
  async cleanupOrphans(live: Set<string>) {
    const root = this.root as FileSystemDirectoryHandle & {
      entries(): AsyncIterableIterator<[string, FileSystemHandle]>;
    };
    try {
      for await (const [name, handle] of root.entries()) {
        if (
          handle.kind === "directory" &&
          /^[0-9a-f-]{36}$/i.test(name) &&
          !live.has(name)
        )
          await this.root
            .removeEntry(name, { recursive: true })
            .catch(() => {});
      }
    } catch {
      /* Retry failed cleanup on the next launch. */
    }
  }
  async removeJob(id: string) {
    await new Promise<void>((resolve, reject) => {
      const tx = this.db.transaction("jobs", "readwrite");
      tx.objectStore("jobs").delete(id);
      tx.oncomplete = () => resolve();
      tx.onerror = tx.onabort = () => reject(tx.error);
    });
    try {
      await this.root.removeEntry(id, { recursive: true });
      return true;
    } catch (error) {
      return error instanceof DOMException && error.name === "NotFoundError";
    }
  }
  async cleanBuild(id: string, keep?: string) {
    const dir = (await this.dir(id)) as FileSystemDirectoryHandle & {
      entries(): AsyncIterableIterator<[string, FileSystemHandle]>;
    };
    const obsolete: string[] = [];
    for await (const [name, handle] of dir.entries()) {
      if (
        handle.kind === "file" &&
        /^archive-r\d+\.zim$/.test(name) &&
        name !== keep
      )
        obsolete.push(name);
    }
    for (const name of obsolete) await dir.removeEntry(name);
    return obsolete.length;
  }
  async dir(id: string) {
    return this.root.getDirectoryHandle(id, { create: true });
  }
}
