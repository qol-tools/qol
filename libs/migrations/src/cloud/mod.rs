//! PostAuth-phase backend abstractions live here.
//!
//! These modules sit behind traits so that migrations (and their tests) can
//! exercise cloud-storage flows without hitting real network services. The
//! production wiring uses the GitHub-backed implementations; tests use the
//! in-memory ones.

pub mod gist_store;
