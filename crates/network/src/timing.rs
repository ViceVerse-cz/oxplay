// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in aggregate diagnostics; never accepts a URL, header, offset, or ID.
use crate::Error;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Copy)]
pub(super) enum Trigger {
    Read,
    Size,
}
// Fixed counters only: HTTP status values are grouped, never recorded verbatim.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ErrorCounts {
    policy: u64,
    dns_configuration: u64,
    status_auth_denied: u64,
    status_range_unsatisfiable: u64,
    status_rate_limited: u64,
    status_other_client: u64,
    status_server: u64,
    status_other: u64,
    cancelled: u64,
    access_revoked: u64,
    timeout: u64,
    transport: u64,
    invalid_response: u64,
    unsupported: u64,
    abandoned: u64,
}
impl ErrorCounts {
    fn record(&mut self, error: Option<Error>) {
        let count = match error {
            Some(Error::Policy) => &mut self.policy,
            Some(Error::DnsConfiguration) => &mut self.dns_configuration,
            Some(Error::HttpStatus(401 | 403)) => &mut self.status_auth_denied,
            Some(Error::HttpStatus(416)) => &mut self.status_range_unsatisfiable,
            Some(Error::HttpStatus(429)) => &mut self.status_rate_limited,
            Some(Error::HttpStatus(400..=499)) => &mut self.status_other_client,
            Some(Error::HttpStatus(500..=599)) => &mut self.status_server,
            Some(Error::HttpStatus(_)) => &mut self.status_other,
            Some(Error::Cancelled) => &mut self.cancelled,
            Some(Error::AccessRevoked) => &mut self.access_revoked,
            Some(Error::Timeout) => &mut self.timeout,
            Some(Error::Transport) => &mut self.transport,
            Some(Error::InvalidResponse) => &mut self.invalid_response,
            Some(Error::Unsupported) => &mut self.unsupported,
            None => &mut self.abandoned,
        };
        *count = count.saturating_add(1);
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    starts: u64,
    completions: u64,
    errors: u64,
    cancellations: u64,
    error_categories: ErrorCounts,
    validated_bytes: u64,
    total_us: u64,
    max_us: u64,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Totals {
    read: Counts,
    size: Counts,
    first_completion_us: Option<u64>,
    first_error_us: Option<u64>,
}
impl Totals {
    fn counts(&mut self, trigger: Trigger) -> &mut Counts {
        match trigger {
            Trigger::Read => &mut self.read,
            Trigger::Size => &mut self.size,
        }
    }
    fn complete(
        &mut self,
        trigger: Trigger,
        elapsed: u64,
        since_first_start: u64,
        result: Option<Result<usize, Error>>,
    ) {
        self.first_completion_us.get_or_insert(since_first_start);
        if !matches!(result, Some(Ok(_))) {
            self.first_error_us.get_or_insert(since_first_start);
        }
        let counts = self.counts(trigger);
        counts.completions = counts.completions.saturating_add(1);
        counts.total_us = counts.total_us.saturating_add(elapsed);
        counts.max_us = counts.max_us.max(elapsed);
        match result {
            Some(Ok(bytes)) => {
                counts.validated_bytes = counts.validated_bytes.saturating_add(bytes as u64)
            }
            failure => {
                let error = failure.and_then(Result::err);
                counts.errors = counts.errors.saturating_add(1);
                counts.cancellations = counts
                    .cancellations
                    .saturating_add(u64::from(error == Some(Error::Cancelled)));
                counts.error_categories.record(error);
            }
        }
    }
}
#[derive(Default)]
struct State {
    first_start: Option<Instant>,
    totals: Totals,
}
#[derive(Default)]
pub(super) struct Stats(Mutex<State>);
impl Stats {
    pub fn configured() -> Option<Arc<Self>> {
        std::env::var_os("SEREIN_HTTP_TIMING")
            .is_some_and(|value| value == "1")
            .then(|| Arc::new(Self::default()))
    }
    pub fn begin(self: &Arc<Self>, trigger: Trigger) -> Attempt {
        let now = Instant::now();
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let first_start = *state.first_start.get_or_insert(now);
        let counts = state.totals.counts(trigger);
        counts.starts = counts.starts.saturating_add(1);
        Attempt {
            stats: self.clone(),
            trigger,
            start: now,
            first_start,
            result: None,
        }
    }
}
impl Drop for Stats {
    fn drop(&mut self) {
        let state = self.0.get_mut().unwrap_or_else(|e| e.into_inner());
        eprintln!(
            "HTTP range timing (one released source; microseconds): {:?}",
            state.totals
        );
    }
}
pub(super) struct Attempt {
    stats: Arc<Stats>,
    trigger: Trigger,
    start: Instant,
    first_start: Instant,
    result: Option<Result<usize, Error>>,
}
impl Attempt {
    pub fn finish(mut self, result: Result<usize, Error>) {
        self.result = Some(result);
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        let now = Instant::now();
        let micros =
            |since| u64::try_from(now.duration_since(since).as_micros()).unwrap_or(u64::MAX);
        self.stats
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .totals
            .complete(
                self.trigger,
                micros(self.start),
                micros(self.first_start),
                self.result,
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_separates_triggers_errors_and_cancellation_without_sensitive_fields() {
        let mut totals = Totals::default();
        totals.read.starts = 2;
        totals.size.starts = 1;
        totals.complete(Trigger::Read, 10, 10, Some(Ok(64)));
        totals.complete(Trigger::Size, 20, 25, Some(Err(Error::Cancelled)));
        totals.complete(Trigger::Read, 30, 50, Some(Err(Error::HttpStatus(403))));
        assert_eq!(
            totals.read,
            Counts {
                starts: 2,
                completions: 2,
                errors: 1,
                cancellations: 0,
                error_categories: ErrorCounts {
                    status_auth_denied: 1,
                    ..ErrorCounts::default()
                },
                validated_bytes: 64,
                total_us: 40,
                max_us: 30
            }
        );
        assert_eq!(totals.size.cancellations, 1);
        assert_eq!(totals.first_completion_us, Some(10));
        assert_eq!(totals.first_error_us, Some(25));
        let text = format!("{totals:?}");
        for forbidden in ["http", "403", "host", "header", "offset", "url", "token"] {
            assert!(!text.contains(forbidden));
        }
    }
    #[test]
    fn categories_are_bounded_exhaustive_and_status_values_are_not_retained() {
        let errors = [
            Error::Policy,
            Error::DnsConfiguration,
            Error::HttpStatus(401),
            Error::HttpStatus(403),
            Error::HttpStatus(416),
            Error::HttpStatus(429),
            Error::HttpStatus(404),
            Error::HttpStatus(500),
            Error::HttpStatus(599),
            Error::HttpStatus(200),
            Error::HttpStatus(302),
            Error::Cancelled,
            Error::AccessRevoked,
            Error::Timeout,
            Error::Transport,
            Error::InvalidResponse,
            Error::Unsupported,
        ];
        let mut totals = Totals::default();
        for (index, error) in errors.iter().enumerate() {
            totals.complete(Trigger::Read, 1, index as u64 + 10, Some(Err(*error)));
        }
        assert_eq!(totals.read.errors, errors.len() as u64);
        assert_eq!(totals.read.cancellations, 1);
        assert_eq!(totals.first_error_us, Some(10));
        assert_eq!(
            totals.read.error_categories,
            ErrorCounts {
                policy: 1,
                dns_configuration: 1,
                status_auth_denied: 2,
                status_range_unsatisfiable: 1,
                status_rate_limited: 1,
                status_other_client: 1,
                status_server: 2,
                status_other: 2,
                cancelled: 1,
                access_revoked: 1,
                timeout: 1,
                transport: 1,
                invalid_response: 1,
                unsupported: 1,
                abandoned: 0,
            }
        );
        let text = format!("{:?}", totals.read.error_categories);
        for error in errors {
            if let Error::HttpStatus(status) = error {
                assert!(!text.contains(&status.to_string()));
            }
        }
        let mut saturated = ErrorCounts {
            policy: u64::MAX,
            ..ErrorCounts::default()
        };
        saturated.record(Some(Error::Policy));
        assert_eq!(saturated.policy, u64::MAX);
    }
    #[test]
    fn incomplete_attempt_counts_error_and_owns_stats_until_completion() {
        let stats = Arc::new(Stats::default());
        let weak = Arc::downgrade(&stats);
        let attempt = stats.begin(Trigger::Read);
        drop(stats);
        assert!(weak.upgrade().is_some());
        let hold = weak.upgrade().unwrap();
        drop(attempt);
        let totals = hold.0.lock().unwrap().totals;
        assert_eq!(totals.read.starts, 1);
        assert_eq!(totals.read.completions, 1);
        assert_eq!(totals.read.errors, 1);
        assert_eq!(totals.read.error_categories.abandoned, 1);
        assert_eq!(totals.read.error_categories.transport, 0);
        assert_eq!(totals.first_error_us, totals.first_completion_us);
        drop(hold);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn cancelled_fetch_is_counted_without_io_and_last_reader_owns_aggregate() {
        let source = crate::HttpSource {
            url: "https://synthetic.googlevideo.com/videoplayback?secret=synthetic-secret"
                .parse()
                .unwrap(),
            headers: reqwest::header::HeaderMap::new(),
            timing: Some(Arc::new(Stats::default())),
            access: None,
            config: crate::NetworkConfig::new(Some("/synthetic/serein-dns".into())).unwrap(),
        };
        let weak = Arc::downgrade(source.timing.as_ref().unwrap());
        let (mut reader, cancel) = source.open();
        let other_source = source.clone();
        drop(source);
        drop(other_source);
        assert!(weak.upgrade().is_some());
        cancel.cancel();
        assert_eq!(reader.fetch(0, Trigger::Read), Err(Error::Cancelled));
        assert!(reader.session.is_none());
        let totals = weak.upgrade().unwrap().0.lock().unwrap().totals;
        assert_eq!(totals.read.starts, 1);
        assert_eq!(totals.read.completions, 1);
        assert_eq!(totals.read.errors, 1);
        assert_eq!(totals.read.cancellations, 1);
        assert_eq!(totals.read.validated_bytes, 0);
        assert!(!format!("{totals:?}").contains("synthetic"));
        drop(reader);
        assert!(weak.upgrade().is_none());
    }
}
