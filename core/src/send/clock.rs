//! The queue's notion of "now". Production reads the system clock; tests use a
//! clock they move by hand, so undo windows, backoff and the 24 hour limit
//! can be exercised without waiting and without pausing the async runtime
//! (which would also fast-forward the fake servers' socket timeouts).
use chrono::{DateTime, SecondsFormat, Utc};

#[derive(Clone, Default)]
pub(crate) enum Clock {
    #[default]
    System,
    #[cfg(test)]
    Manual(std::sync::Arc<std::sync::Mutex<DateTime<Utc>>>),
}

impl Clock {
    pub(crate) fn now(&self) -> DateTime<Utc> {
        match self {
            Self::System => Utc::now(),
            #[cfg(test)]
            Self::Manual(time) => *time.lock().unwrap(),
        }
    }

    /// The longest a worker sleeps before looking at the queue again. A moved
    /// system clock cannot strand an item for long; a manual clock does not
    /// move by itself, so tests poll quickly.
    pub(crate) fn poll_cap(&self) -> std::time::Duration {
        match self {
            Self::System => std::time::Duration::from_secs(60),
            #[cfg(test)]
            Self::Manual(_) => std::time::Duration::from_millis(10),
        }
    }

    #[cfg(test)]
    pub(crate) fn manual(start: DateTime<Utc>) -> Self {
        Self::Manual(std::sync::Arc::new(std::sync::Mutex::new(start)))
    }

    #[cfg(test)]
    pub(crate) fn advance(&self, by: std::time::Duration) {
        if let Self::Manual(time) = self {
            let mut time = time.lock().unwrap();
            *time += chrono::Duration::from_std(by).unwrap();
        }
    }
}

/// RFC 3339 UTC with milliseconds; text order is time order.
pub(crate) fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn parse_stamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}
