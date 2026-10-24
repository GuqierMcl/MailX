//! The send queue: snapshots taken at queue time, one serial worker per
//! account, state changes announced as events (docs/architecture/sending.md
//! sections 5 and 6).
pub(crate) mod clock;
mod schedule;
#[cfg(test)]
mod tests;
pub mod types;
pub(crate) mod worker;

pub(crate) use clock::Clock;
pub(crate) use worker::{OutboxManager, SendConfig};
