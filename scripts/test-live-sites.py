#!/usr/bin/env python3
"""Bounded, opt-in live crawl matrix against an isolated local Zimsmith worker."""
import argparse
import concurrent.futures
import json
import time
import urllib.error
import urllib.request
from pathlib import Path

SITES = {
    "Ars Technica": "https://arstechnica.com/",
    "BBC News": "https://www.bbc.com/news",
    "The Guardian": "https://www.theguardian.com/international",
    "CNN": "https://www.cnn.com/",
    "NPR": "https://www.npr.org/",
    "Rust Blog": "https://blog.rust-lang.org/",
    "Simon Willison": "https://simonwillison.net/",
    "Krebs on Security": "https://krebsonsecurity.com/",
    "NASA": "https://www.nasa.gov/",
    "MDN": "https://developer.mozilla.org/en-US/docs/Web/",
}


def api(base, path, data=None):
    request = urllib.request.Request(
        base + path,
        data=None if data is None else json.dumps(data).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def crawl(item, args):
    name, url = item
    started = time.monotonic()
    job = None
    failure = None
    try:
        job = api(args.api, "/jobs", {
            "url": url, "title": name + " compatibility sample", "kind": "website",
            "max_items": 3, "max_depth": 1, "delay_ms": 1000,
            "max_resource_mb": args.resource_mb,
            "language": "eng", "description": "Bounded live-site compatibility test",
            "creator": "Zimsmith", "include_assets": True,
            "capture": {"max_download_mb": args.download_mb, "auto_render_javascript": False},
        })
        while time.monotonic() - started < args.timeout_seconds:
            job = api(args.api, "/jobs/" + job["id"])
            if job["status"] == "review":
                selected = [e["id"] for e in job["entries"]
                            if e["selected"] and e["state"] != "scan_failed"]
                if not selected:
                    break
                try:
                    api(args.api, "/jobs/" + job["id"] + "/build", {"selected": selected})
                except urllib.error.HTTPError as error:
                    if "still running" not in error.read().decode():
                        raise
            elif job["status"] in ("failed", "completed", "paused"):
                break
            time.sleep(1)
    except Exception as error:
        failure = str(error)
    finally:
        # Also pause on harness failures, so an abandoned test cannot keep crawling.
        if job and job["status"] not in ("failed", "completed", "paused", "review"):
            try:
                api(args.api, "/jobs/" + job["id"] + "/pause", {})
                time.sleep(2)
                job = api(args.api, "/jobs/" + job["id"])
            except Exception as error:
                failure = f"{failure or ''} Pause could not be confirmed: {error}".strip()
    if not job:
        return {"site": name, "url": url, "status": "harness_error", "error": failure}
    return {
        "site": name, "url": url, "status": job["status"], "phase": job["phase"],
        "pages": sum(e["state"] != "scan_failed" for e in job["entries"]),
        "requests": job["requests"],
        "downloaded_mb": round(job["downloaded_bytes"] / 1_000_000, 2),
        "zim_mb": round((job.get("output_bytes") or 0) / 1_000_000, 2),
        "seconds": round(time.monotonic() - started), "error": job.get("error"),
        "harness_error": failure, "validation": job.get("validation"),
        "report": job.get("capture_report"), "job_id": job["id"],
    }


def positive_int(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", default="http://127.0.0.1:8793/api")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--site", choices=SITES, action="append", help="Only test these sites")
    parser.add_argument("--resource-mb", type=positive_int, default=4)
    parser.add_argument("--download-mb", type=positive_int, default=30)
    parser.add_argument("--timeout-seconds", type=positive_int, default=150)
    args = parser.parse_args()
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(crawl, (name, SITES[name]), args)
                   for name in (args.site or SITES)]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(results, indent=2) + "\n")
            print(result["site"], result["status"], result.get("pages", 0), "pages",
                  result.get("downloaded_mb", 0), "MB", flush=True)


if __name__ == "__main__":
    main()
