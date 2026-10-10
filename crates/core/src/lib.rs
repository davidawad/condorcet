//! assaymark core: content-addressed ids and the `{ok, data | error}` envelope.
//!
//! The Bundle, composer, profile and run types come with the beads that need
//! them; the JSON schemas under `schemas/` are the authoritative contract for
//! now.

pub mod envelope;
pub mod id;

pub use envelope::{Envelope, ErrorBody};
pub use id::{canonical_json, content_id, IdKind};
