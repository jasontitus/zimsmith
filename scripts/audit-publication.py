#!/usr/bin/env python3
"""Check the exact staged tree and release archives before public upload.

Outputs filenames/categories/line numbers, never matching secret contents.
Run Gitleaks separately for broad credential detection; this complements it
with local-path, private-file, device-identifier and archive-member checks.
"""
import argparse
import hashlib
import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--artifact", action="append", default=[])
parser.add_argument("--private-identifiers-file", type=Path)
parser.add_argument("--report", type=Path)
args = parser.parse_args()
private = json.loads(args.private_identifiers_file.read_text()) if args.private_identifiers_file else []
patterns = {
    "personal-home-path": re.compile(re.escape(b"/" + b"Users/") + rb"[^/\x00\s]{1,80}/|[A-Za-z]:\\Users\\[^\\\x00\s]{1,80}\\|" + re.escape(b"/" + b"home/") + rb"(?!runner/)[^/\x00\s]{1,80}/"),
    "private-key": re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----"),
    "github-credential": re.compile(rb"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{35,})"),
    "apple-device-identifier": re.compile(rb"\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{16}\b"),
}
for index, value in enumerate(private):
    if value:
        patterns[f"private-identifier-{index + 1}"] = re.compile(re.escape(value.encode()), re.I)
blocked = re.compile(r"(?:^|/)(?:\.env(?:\..*)?|\.signing|secrets|\.zim-studio|diagnostics|node_modules|output|target|\.tools|test-results|playwright-report)(?:/|$)|\.(?:p12|pfx|pem|key|keystore|jks|mobileprovision|tsbuildinfo)$|^src-tauri/gen/", re.I)
findings = []
manifest = []

def scan(label, data):
    for category, pattern in patterns.items():
        match = pattern.search(data)
        if match:
            findings.append({"file": label, "category": category, "line": data[:match.start()].count(b"\n") + 1})

files = subprocess.check_output(["git", "ls-files", "--stage", "-z"]).decode().split("\0")
for entry in filter(None, files):
    header, name = entry.split("\t", 1)
    mode, oid, stage = header.split()
    if stage != "0" or mode not in {"100644", "100755"}:
        findings.append({"file": name, "category": "unresolved-or-symlink", "line": 0})
        continue
    if blocked.search(name) and name != ".env.example":
        findings.append({"file": name, "category": "private-or-generated-file", "line": 0})
    data = subprocess.check_output(["git", "cat-file", "blob", oid])
    scan(name, data)
    manifest.append({"path": name, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
for filename in args.artifact:
    artifact = Path(filename)
    data = artifact.read_bytes()
    scan(artifact.name, data)
    manifest.append({"path": artifact.name, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
    if zipfile.is_zipfile(artifact):
        with zipfile.ZipFile(artifact) as archive:
            for member in archive.infolist():
                if member.is_dir():
                    continue
                if member.file_size > 256 * 1024 * 1024:
                    findings.append({"file": f"{artifact.name}:{member.filename}", "category": "member-exceeds-review-limit", "line": 0})
                else:
                    scan(f"{artifact.name}:{member.filename}", archive.read(member))
report = {"reviewed_files": len(manifest), "findings": findings, "manifest": manifest}
if args.report:
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({"reviewed_files": len(manifest), "findings": findings}, indent=2))
sys.exit(bool(findings))
