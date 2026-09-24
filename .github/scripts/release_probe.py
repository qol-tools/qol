#!/usr/bin/env python3
"""Compare a freshly built plugin binary with its last released asset.

A plugin reached only through shared crates is released only when this
reports a difference. Linux binaries are compared after blanking the two
parts that change without changing behaviour: the GNU build id (a hash of the
whole file) and exception-table bytes that no unwind entry references (lld
keeps the tables of functions it discarded). Any other difference, a layout
shift included, counts as changed. Other platforms compare raw bytes, and
anything that cannot be compared counts as changed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ELF_MAGIC = b"\x7fELF"
OMITTED = 0xFF
SECTION_RE = re.compile(
    r"\]\s+(\S+)\s+\S+\s+([0-9a-f]+)\s+([0-9a-f]+)\s+([0-9a-f]+)"
)
FDE_RE = re.compile(
    r"^([0-9a-f]+) [0-9a-f]+ [0-9a-f]+ FDE .*\n\s+Augmentation data:\s+((?:[0-9a-f]{2} ?)+)",
    re.MULTILINE,
)
FDE_HEADER_BEFORE_LSDA_POINTER = 4 + 4 + 4 + 4 + 1


def read_uleb(data: bytes, index: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[index]
        index += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, index


def read_sleb(data: bytes, index: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[index]
        index += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            if byte & 0x40:
                value -= 1 << shift
            return value, index


def lsda_end(data: bytes, start: int) -> int:
    """End offset of the LSDA at `start`: call sites, reachable actions, types."""
    if data[start] != OMITTED:
        raise ValueError(f"unsupported LSDA landing pad base at {start:#x}")
    index = start + 1
    type_encoding = data[index]
    index += 1
    types_end = None
    if type_encoding != OMITTED:
        offset, index = read_uleb(data, index)
        types_end = index + offset
    index += 1
    length, index = read_uleb(data, index)
    call_sites_end = index + length
    actions: list[int] = []
    while index < call_sites_end:
        for _ in range(3):
            _, index = read_uleb(data, index)
        action, index = read_uleb(data, index)
        if action:
            actions.append(call_sites_end + action - 1)
    end = call_sites_end
    seen: set[int] = set()
    while actions:
        record = actions.pop()
        if record in seen:
            continue
        seen.add(record)
        _, next_field = read_sleb(data, record)
        offset, after = read_sleb(data, next_field)
        end = max(end, after)
        if offset:
            actions.append(next_field + offset)
    return max(end, types_end) if types_end is not None else end


def elf_sections(path: Path) -> dict[str, tuple[int, int, int]]:
    output = subprocess.run(
        ["readelf", "-S", "-W", str(path)], capture_output=True, text=True, check=True
    ).stdout
    return {
        match.group(1): (
            int(match.group(2), 16),
            int(match.group(3), 16),
            int(match.group(4), 16),
        )
        for match in SECTION_RE.finditer(output)
    }


def referenced_lsdas(path: Path, eh_frame_address: int) -> list[int]:
    output = subprocess.run(
        ["readelf", "--debug-dump=frames", str(path)],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    addresses = []
    for match in FDE_RE.finditer(output):
        field = bytes.fromhex(match.group(2).replace(" ", ""))
        if len(field) != 4:
            raise ValueError("unsupported LSDA pointer encoding")
        fde = eh_frame_address + int(match.group(1), 16)
        addresses.append(fde + FDE_HEADER_BEFORE_LSDA_POINTER + struct.unpack("<i", field)[0])
    return addresses


def blank_unreferenced(data: bytearray, offset: int, size: int, live: list[tuple[int, int]]) -> None:
    keep = bytearray(size)
    for start, end in live:
        keep[start - offset : end - offset] = b"\x01" * (end - start)
    for index, used in enumerate(keep):
        if not used:
            data[offset + index] = 0


def normalized_elf(path: Path) -> bytes:
    data = bytearray(path.read_bytes())
    sections = elf_sections(path)
    if ".note.gnu.build-id" in sections:
        _, offset, size = sections[".note.gnu.build-id"]
        data[offset : offset + size] = bytes(size)
    if ".gcc_except_table" in sections:
        address, offset, size = sections[".gcc_except_table"]
        live = []
        for lsda in referenced_lsdas(path, sections[".eh_frame"][0]):
            start = lsda - address + offset
            live.append((start, lsda_end(data, start)))
        blank_unreferenced(data, offset, size, live)
    return bytes(data)


def binary_digest(path: Path) -> str:
    data = path.read_bytes()
    if data.startswith(ELF_MAGIC):
        data = normalized_elf(path)
    return hashlib.sha256(data).hexdigest()


def download_asset(repo: str, tag: str, asset: str, directory: Path) -> Path | None:
    result = subprocess.run(
        [
            "gh", "release", "download", tag,
            "--repo", repo,
            "--pattern", asset,
            "--dir", str(directory),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    downloaded = directory / asset
    if result.returncode != 0 or not downloaded.is_file():
        print(result.stderr.strip(), file=sys.stderr)
        return None
    return downloaded


def compare(binary: Path, previous: Path | None, report: dict) -> dict:
    if previous is None:
        report["reason"] = "the last release has no asset for this target"
        return report
    report["current"] = binary_digest(binary)
    report["previous"] = binary_digest(previous)
    report["identical"] = report["current"] == report["previous"]
    report["reason"] = "binary unchanged" if report["identical"] else "binary changed"
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--unit", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--previous-tag", required=True)
    parser.add_argument("--asset", required=True)
    parser.add_argument("--report", required=True)
    args = parser.parse_args()
    report = {
        "unit": args.unit,
        "target": args.target,
        "previous_tag": args.previous_tag,
        "identical": False,
        "reason": "",
    }
    with tempfile.TemporaryDirectory() as scratch:
        previous = download_asset(args.repo, args.previous_tag, args.asset, Path(scratch))
        compare(Path(args.binary), previous, report)
    Path(args.report).write_text(json.dumps(report, indent=2) + "\n")
    print(f"{args.unit} ({args.target}): {report['reason']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
