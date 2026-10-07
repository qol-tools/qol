<div align="center">

# QoL Work Queue

One keyed work queue with a single worker for qol-tools.

</div>

## What it does

- Every task has a key. A key that is already waiting or running is refused, so the same work is never queued twice.
- One worker runs the tasks one at a time, oldest first.
- A task pushed with `Run::Alone` waits until nothing else is waiting, and while it runs every new push is refused. Use it for work that ends the process, such as an update that restarts the app.
- A waiting task can be cancelled. A failed task keeps its reason until the same key is pushed again.
- `snapshot` and `get` show every task with its state, progress and failure reason.

## Quick start

```toml
[dependencies]
qol-work-queue.workspace = true
```

```rust
use qol_work_queue::{Run, WorkQueue};

let queue = WorkQueue::new();
queue.push("plugin-a", "update", Run::InOrder)?;
queue.push("app", "restart", Run::Alone)?;
tokio::spawn(async move {
    queue.run(|key, task| async move { do_work(&key, task).await }).await
});
```
