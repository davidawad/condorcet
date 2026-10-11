//! condorcet core: content-addressed ids, the `{ok, data | error}` envelope,
//! the Bundle (adapter interchange) and the run record.

pub mod bundle;
pub mod envelope;
pub mod id;
pub mod run;

pub use bundle::{
    AdapterProvenance, Bundle, BundleError, Claim, Cluster, Constraint, ConstraintKind, Exhibit, Link,
    LinkKind, Mode, Reading, Scope, Weight, BUNDLE_SCHEMA,
};
pub use envelope::{Envelope, ErrorBody};
pub use id::{canonical_json, content_id, IdKind};
pub use run::{Evidence, GateRecord, NameVersion, RunRecord, StepError, StepRecord, RUN_SCHEMA};
