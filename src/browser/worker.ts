import { BrowserEngine } from "./engine";

const ready = BrowserEngine.open();
// Install the rejection handler immediately, even if initialization finishes
// before the first RPC arrives. Every request receives the same useful error.
void ready.catch(() => {});
self.addEventListener("message", async (event) => {
  const { id, command, args } = event.data;
  try {
    self.postMessage({ id, value: await (await ready).call(command, args) });
  } catch (error) {
    self.postMessage({
      id,
      error: error instanceof Error ? error.message : String(error),
    });
  }
});
