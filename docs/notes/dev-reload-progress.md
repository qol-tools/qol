# Development reload progress

The dashboard reload activity covers prebuild and the entire successor handoff.
Successful prebuild completion moves the existing progress into the handoff state,
preserving its start time until staging, readiness, predecessor retirement,
promotion, cleanup, and autostart repair finish. Phase transitions appear in the
activity line and existing reload logs with total elapsed milliseconds.

Handoff retains the calling thread for child spawning and process supervision.
On Linux, the parent-death signal follows the thread that spawned the child;
moving the handoff into a temporary worker would kill the successor when that
worker exits. During handoff, a scoped display thread exclusively borrows the
dashboard and terminal, applies queued updates, and renders on the normal tick.
It joins before the session resumes. Keyboard input remains queued during this
phase. Display failures are reported after handoff cleanup completes.

The regression fixtures exercise successful child completion, real dashboard
rendering across a blocked handoff, the original elapsed-time clock and caller
thread, worktree adoption, and both handoff and terminal failures. They use no
resident tray endpoints. Desktop reload smoke tests belong in a disposable guest.

Linux ownership reference: [PR_SET_PDEATHSIG](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html).
