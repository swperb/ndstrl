//! NDSTRL core: the job rules every agent works under.
//!
//! * [`spec`] the job contract
//! * [`scope`] workholding: what may be read and cut
//! * [`reads`] search and excerpts served by NDSTRL
//! * [`checks`] local compile and test
//! * [`meter`] real spend from the harness transcript, plus NDSTRL's own
//! * [`plan`] subscription window usage from the status line
//! * [`job`] the engine that enforces all of it

pub mod checks;
pub mod job;
pub mod meter;
pub mod plan;
pub mod reads;
pub mod scope;
pub mod spec;

pub use job::{Gauges, Job, Outcome, Question};
pub use spec::JobSpec;
