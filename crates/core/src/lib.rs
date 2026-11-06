//! NDSTRL core: the factory every machine talks to.
//!
//! * [`traveler`] the job contract
//! * [`scope`] workholding: what may be read and cut
//! * [`reads`] factory-owned search and excerpts
//! * [`critic`] local compile and test
//! * [`meter`] real spend from the harness transcript, plus the factory's own
//! * [`plan`] subscription window usage from the status line
//! * [`job`] the engine that enforces all of it

pub mod critic;
pub mod job;
pub mod meter;
pub mod plan;
pub mod reads;
pub mod scope;
pub mod traveler;

pub use job::{Gauges, Job, Outcome, Question};
pub use traveler::Traveler;
