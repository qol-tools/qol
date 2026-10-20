#!/usr/bin/env python3
import os
import select
import sys


def stat_fields(pid):
    with open(f"/proc/{pid}/stat") as handle:
        return handle.read().rsplit(") ", 1)[1].split()


def main():
    iterations = int(sys.argv[1])
    root = os.environ["QOL_PROCESS_CGROUP_ROOT"]
    cgroup = os.path.join(root, f"exit-window-{os.getpid()}")
    os.mkdir(cgroup)
    procs = os.path.join(cgroup, "cgroup.procs")
    hits = 0
    examples = []
    try:
        for _ in range(iterations):
            ready_read, ready_write = os.pipe()
            pid = os.fork()
            if pid == 0:
                os.close(ready_read)
                with open(procs, "w") as handle:
                    handle.write("0\n")
                os.write(ready_write, b"1")
                os.execv("/bin/true", ["true"])
            os.close(ready_write)
            os.read(ready_read, 1)
            os.close(ready_read)
            pidfd = os.pidfd_open(pid)
            seen = False
            while True:
                with open(procs) as handle:
                    listed = str(pid) in handle.read().split()
                exited = bool(select.select([pidfd], [], [], 0)[0])
                if exited:
                    break
                if not listed and not seen:
                    seen = True
                    hits += 1
                    try:
                        fields = stat_fields(pid)
                        flags = int(fields[6])
                        with open(f"/proc/{pid}/cgroup") as handle:
                            membership = handle.read().strip()
                        examples.append(
                            f"state={fields[0]} flags={flags:#x} "
                            f"pf_exiting={bool(flags & 0x4)} proc_cgroup={membership}"
                        )
                    except OSError as error:
                        examples.append(f"read failed: {error}")
            os.close(pidfd)
            os.waitpid(pid, 0)
    finally:
        os.rmdir(cgroup)
    print(
        f"exit-window probe: {iterations} children, {hits} were absent from "
        f"cgroup.procs while their pidfd still reported them alive"
    )
    for example in examples[:10]:
        print(f"  {example}")


if __name__ == "__main__":
    main()
