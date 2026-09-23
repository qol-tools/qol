import os
from pathlib import Path
import sys


def cleanup(journal_root: Path, cgroup_root: Path) -> None:
    if os.path.lexists(cgroup_root):
        raise RuntimeError("refusing to remove journals before the cgroup tree is gone")
    if journal_root.is_symlink() or not journal_root.is_dir():
        raise RuntimeError("journal root must be a directory, not a symlink")
    entries = list(journal_root.iterdir())
    if any(entry.is_symlink() or not entry.is_file() for entry in entries):
        raise RuntimeError("journal root contains an unexpected entry")
    for entry in entries:
        entry.unlink()
    journal_root.rmdir()


if __name__ == "__main__":
    cleanup(Path(sys.argv[1]), Path(sys.argv[2]))
