//! Pure data transforms used by the cloud-migration code path.
//!
//! Transforms are stateless functions that convert one serialized representation
//! of a profile into another (e.g. a legacy gist JSON into the per-file on-disk
//! layout). They do no I/O and return an in-memory `HashMap<PathBuf, Vec<u8>>`
//! that the caller is free to write, ship over the network, or diff against an
//! existing tree.

pub mod gist_v1_to_layout;
