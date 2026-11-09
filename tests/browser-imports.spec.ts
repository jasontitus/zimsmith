import { test, expect } from "@playwright/test";
import { createServer } from "node:http";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join, extname } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { existsSync } from "node:fs";

test.skip(
  !existsSync("output/web/index.html"),
  "Build the browser version first.",
);
for (const format of ["ZIP", "WARC.GZ"])
  test(`browser imports a saved ${format} without networking and exports a validated ZIM`, async ({
    page,
  }) => {
    test.setTimeout(120000);
    const temp = await mkdtemp(join(tmpdir(), "zimsmith-import-test-")),
      root = resolve("output/web");
    const server = createServer(async (req, res) => {
      const name = resolve(
        root,
        "." + new URL(req.url || "/", "http://local").pathname,
      );
      if (!name.startsWith(root + "/") && name !== root) {
        res.writeHead(404).end();
        return;
      }
      try {
        const path = name === root ? join(root, "index.html") : name;
        const bytes = await readFile(path);
        res.setHeader(
          "Content-Type",
          (
            {
              ".html": "text/html",
              ".js": "text/javascript",
              ".css": "text/css",
              ".woff2": "font/woff2",
              ".png": "image/png",
            } as Record<string, string>
          )[extname(path)] || "application/octet-stream",
        );
        res.end(bytes);
      } catch {
        res.writeHead(404).end();
      }
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("No fixture port");
    const origin = `http://127.0.0.1:${address.port}`;
    const requests: string[] = [],
      errors: string[] = [];
    page.on("request", (req) => requests.push(req.url()));
    page.on("pageerror", (error) => errors.push(error.message));
    try {
      await promisify(execFile)("python3", [
        "-c",
        `import zipfile,sys,gzip,io
archive=io.BytesIO()
with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED) as z:
 z.writestr('pages/index.html',"<title>Saved botany</title><link rel='stylesheet' href='assets/style.css'><p>Moonlight gardens and botanical notes.</p><a href='guide.html'>Guide</a><img src='https://unavailable.invalid/missing.png'>")
 z.writestr('pages/guide.html',"<title>Garden guide</title><p>Details from the saved garden.</p>")
 z.writestr('pages/assets/style.css',"body{color:#123;background-image:url('leaf.svg')}")
 z.writestr('pages/assets/leaf.svg',"<svg xmlns='http://www.w3.org/2000/svg'/>")
if sys.argv[1].endswith('.zip'):
 open(sys.argv[1],'wb').write(archive.getvalue())
else:
 with zipfile.ZipFile(archive) as z, open(sys.argv[1],'wb') as out:
  for i,name in enumerate(z.namelist()):
   body=z.read(name)
   mime={'html':'text/html','css':'text/css','svg':'image/svg+xml'}[name.rsplit('.',1)[-1]]
   header=f"WARC/1.1\\r\\nWARC-Type: resource\\r\\nWARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-{i:012d}>\\r\\nWARC-Date: 2026-01-01T00:00:00Z\\r\\nWARC-Target-URI: https://capture.invalid/{name}\\r\\nContent-Type: {mime}\\r\\nContent-Length: {len(body)}\\r\\n\\r\\n".encode()
   out.write(gzip.compress(header+body+b"\\r\\n\\r\\n"))`,
        join(temp, format === "ZIP" ? "saved.zip" : "saved.warc.gz"),
      ]);
      await page.goto(origin + "/");
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      await expect(
        page.getByLabel("Zstandard compression level", { exact: true }),
      ).toHaveCount(0);
      await expect(page.locator(".compression-preferences")).toContainText(
        "This browser build writes uncompressed ZIMs",
      );
      await page
        .getByRole("button", { name: "Create an archive", exact: true })
        .first()
        .click();
      const panel = page.locator(".import-panel");
      await panel.locator("summary").click();
      await panel
        .getByLabel("Choose saved files", { exact: true })
        .setInputFiles(
          join(temp, format === "ZIP" ? "saved.zip" : "saved.warc.gz"),
        );
      await panel
        .getByLabel("Imported collection title", { exact: true })
        .fill("Saved garden");
      await panel
        .getByRole("button", { name: "Import & review", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Saved garden", exact: true }),
      ).toBeVisible();
      await expect(
        page.getByText("Imported local files", { exact: true }),
      ).toBeVisible();
      await page
        .getByRole("button", {
          name: "Review & build",
          exact: true,
        })
        .filter({ hasText: /./ })
        .first()
        .click();
      const confirmation = page.getByRole("dialog");
      await confirmation
        .getByRole("button", {
          name: /Download.*build|Build.*archive|Build.*ZIM|Create.*archive/i,
        })
        .click();
      await expect(
        page.getByRole("heading", {
          name: "Your collection is ready to go.",
          exact: true,
        }),
      ).toBeVisible({ timeout: 30000 });
      const report = page.getByRole("region", {
        name: "Capture & validation report",
      });
      await expect(report).toContainText(
        "Checksum and directory structure verified",
      );
      await expect(report).toContainText("Omitted assets");
      expect(requests.filter((url) => !url.startsWith(origin))).toEqual([]);
      const download = page.waitForEvent("download");
      await page
        .getByRole("button", { name: "Save ZIM file", exact: true })
        .click();
      const saved = await download,
        path = join(temp, "garden.zim");
      await saved.saveAs(path);
      await promisify(execFile)(
        "cargo",
        [
          "run",
          "-p",
          "zim-engine",
          "--example",
          "check_browser_zim",
          "--locked",
          "--",
          "" + path,
        ],
        { timeout: 90000 },
      );
      if (format === "ZIP") {
        const id = await page.evaluate(async () => {
          const root = await (
            await navigator.storage.getDirectory()
          ).getDirectoryHandle("zimsmith");
          const dirs = root as FileSystemDirectoryHandle & {
            keys(): AsyncIterableIterator<string>;
          };
          const { value: id } = await dirs.keys().next();
          const dir = await root.getDirectoryHandle(id);
          await dir.getFileHandle("archive-r99.zim", { create: true });
          return id as string;
        });
        await page.getByRole("button", { name: "Clean build files" }).click();
        await expect(
          page.getByText("Temporary build files removed.", { exact: false }),
        ).toBeVisible();
        expect(
          await page.evaluate(async (id) => {
            const root = await (
              await navigator.storage.getDirectory()
            ).getDirectoryHandle("zimsmith");
            const dir = await root.getDirectoryHandle(id);
            const current = await (
              await dir.getFileHandle("archive-r0.zim")
            ).getFile();
            const obsolete = await dir
              .getFileHandle("archive-r99.zim")
              .catch(() => null);
            return current.size > 0 && obsolete === null;
          }, id),
        ).toBe(true);
        await page
          .getByRole("button", { name: /My archives/ })
          .first()
          .click();
        await page
          .getByRole("button", {
            name: "Delete archive for Saved garden",
            exact: true,
          })
          .click();
        const deleteDialog = page.getByRole("dialog", {
          name: "Delete archive?",
        });
        await expect(deleteDialog).toContainText(
          "Exported ZIM copies stay unchanged",
        );
        await deleteDialog
          .getByRole("button", { name: "Delete archive", exact: true })
          .click();
        await expect(
          page.getByRole("heading", { name: "Your offline library." }),
        ).toBeVisible();
        await page.reload();
        await page
          .getByRole("button", { name: /My archives/ })
          .first()
          .click();
        await expect(
          page.getByRole("heading", { name: "Your offline library." }),
        ).toBeVisible();
        await expect(
          page.locator(".job-row").filter({ hasText: "Saved garden" }),
        ).toHaveCount(0);
        expect(
          await page.evaluate(async (id) => {
            const root = await (
              await navigator.storage.getDirectory()
            ).getDirectoryHandle("zimsmith");
            return (
              (await root.getDirectoryHandle(id).catch(() => null)) === null
            );
          }, id),
        ).toBe(true);
        expect((await readFile(path)).length).toBeGreaterThan(0);
      }
      expect(errors).toEqual([]);
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      await rm(temp, { recursive: true, force: true });
    }
  });
