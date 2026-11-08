import { test, expect } from "@playwright/test";
import { createServer, type ViteDevServer } from "vite";

let server: ViteDevServer;
let BrowserEngine: any;
test.beforeAll(async () => {
  server = await createServer({ server: { middlewareMode: true, ws: false } });
  ({ BrowserEngine } = await server.ssrLoadModule("/src/browser/engine.ts"));
});
test.afterAll(async () => {
  await server?.close();
});

function fixture(status = "completed", phase = "package") {
  const job = {
    id: "fixture",
    status,
    phase,
    config: { kind: "website" },
    entries: [{ id: "page", selected: true, state: "done", file: "page.html" }],
    resources: {},
    logs: [],
    archive_revision: 0,
  };
  const engine = new BrowserEngine({ put: async () => {} });
  engine.jobs.set(job.id, job);
  return { engine, job };
}

test("browser commands reject an import whose canceled finalizer still owns files", async () => {
  const { engine, job } = fixture("paused", "import");
  engine.controls.set(job.id, new AbortController());
  await expect(engine.call("resume_job", { id: job.id })).rejects.toThrow(
    /Pause/,
  );
  expect(job.status).toBe("paused");
});

test("browser build and rebuild enforce lifecycle guards", async () => {
  const { engine, job } = fixture();
  await expect(
    engine.call("build_job", { id: job.id, selected: ["page"] }),
  ).rejects.toThrow(/Finish scanning/);
  expect(job.status).toBe("completed");
  job.status = "review";
  job.phase = "scan";
  await expect(
    engine.call("rebuild_job", { id: job.id, template: "atlas" }),
  ).rejects.toThrow(/Rebuild requires/);
  expect(job.status).toBe("review");
});

test("browser refresh reserves the job across asynchronous storage and releases on failure", async () => {
  const { engine, job } = fixture();
  let rejectRead!: (error: Error) => void;
  let reached!: () => void;
  const started = new Promise<void>((resolve) => {
    reached = resolve;
  });
  engine.store.dir = () => {
    reached();
    return new Promise((_, reject) => {
      rejectRead = reject;
    });
  };
  const refresh = engine.call("refresh_job", { id: job.id });
  const failed = expect(refresh).rejects.toThrow("disk unavailable");
  await started;
  await expect(
    engine.call("rebuild_job", { id: job.id, template: "atlas" }),
  ).rejects.toThrow(/in progress/);
  await expect(engine.call("refresh_job", { id: job.id })).rejects.toThrow(
    /in progress/,
  );
  expect(job.status).toBe("completed");
  expect(job.archive_revision).toBe(0);
  rejectRead(new Error("disk unavailable"));
  await failed;
  expect(engine.mutations.size).toBe(0);
  engine.store.dir = async () => {
    throw new Error("retry reached storage");
  };
  await expect(engine.call("refresh_job", { id: job.id })).rejects.toThrow(
    "retry reached storage",
  );
});
