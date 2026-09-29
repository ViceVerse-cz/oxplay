// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite diagnostic query accounting. No timer, URL, credential or audio data.
use crate::{MediaError, Result};

const FIRST_TOKEN: u64 = 1 << 61;
const LIMIT: u64 = 1 << 62;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioProbeReply {
    pub token: u64,
    pub load_request_id: u64,
    pub decoder_sample_rate: Option<i64>,
    pub output_sample_rate: Option<i64>,
    /// Native audio clock in seconds; negative driver-delay values are valid.
    pub audio_pts: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Context {
    pub generation: u64,
    pub load_request_id: u64,
    pub entry: Option<i64>,
}
pub(super) enum Value {
    Integer(i64),
    Float(f64),
    Unavailable,
}
struct Pending {
    context: Context,
    reply: AudioProbeReply,
    expected: u8,
    received: u8,
    cancelled: bool,
}
pub(super) struct AudioProbes {
    next_token: u64,
    pending: Option<Pending>,
}
impl Default for AudioProbes {
    fn default() -> Self {
        Self {
            next_token: FIRST_TOKEN,
            pending: None,
        }
    }
}
pub(super) fn owns_userdata(userdata: u64) -> bool {
    (FIRST_TOKEN..LIMIT).contains(&userdata)
}
impl AudioProbes {
    pub fn begin(&mut self, context: Context) -> Result<u64> {
        if self.pending.is_some() {
            return Err(MediaError("Audio diagnostic query still pending".into()));
        }
        let token = self.next_token;
        self.next_token = token
            .checked_add(4)
            .filter(|next| *next < LIMIT)
            .ok_or_else(|| MediaError("Audio diagnostic identifiers exhausted".into()))?;
        self.pending = Some(Pending {
            context,
            expected: 0,
            received: 0,
            cancelled: false,
            reply: AudioProbeReply {
                token,
                load_request_id: context.load_request_id,
                decoder_sample_rate: None,
                output_sample_rate: None,
                audio_pts: None,
            },
        });
        Ok(token)
    }
    pub fn submitted(&mut self, field: u8) {
        self.pending
            .as_mut()
            .expect("probe reserved before native submission")
            .expected |= 1 << field;
    }
    pub fn abort_submission(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.expected == 0)
        {
            self.pending = None;
        } else {
            self.invalidate();
        }
    }
    pub fn invalidate(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.cancelled = true;
        }
    }
    pub fn cancel(&mut self, token: u64) {
        if let Some(pending) = &mut self.pending
            && pending.reply.token == token
        {
            pending.cancelled = true;
        }
    }
    pub fn receive(
        &mut self,
        userdata: u64,
        value: Value,
        context: Context,
    ) -> Option<AudioProbeReply> {
        let pending = self.pending.as_mut()?;
        let field = userdata.checked_sub(pending.reply.token)?;
        if field >= 3
            || pending.expected & (1 << field) == 0
            || pending.received & (1 << field) != 0
        {
            return None;
        }
        pending.received |= 1 << field;
        pending.cancelled |= pending.context != context;
        match (field, value) {
            (0, Value::Integer(rate)) if rate > 0 => pending.reply.decoder_sample_rate = Some(rate),
            (1, Value::Integer(rate)) if rate > 0 => pending.reply.output_sample_rate = Some(rate),
            (2, Value::Float(pts)) if pts.is_finite() => pending.reply.audio_pts = Some(pts),
            _ => {}
        }
        if pending.received != pending.expected {
            return None;
        }
        let completed = self.pending.take().expect("completed pending probe");
        (!completed.cancelled && completed.expected == 7).then_some(completed.reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> Context {
        Context {
            generation: 3,
            load_request_id: 42,
            entry: Some(7),
        }
    }
    fn submit(probes: &mut AudioProbes) -> u64 {
        let token = probes.begin(context()).unwrap();
        for field in 0..3 {
            probes.submitted(field);
        }
        token
    }
    #[test]
    fn replies_complete_once_out_of_order_without_exposing_partial_values() {
        let mut probes = AudioProbes::default();
        let token = submit(&mut probes);
        assert!(
            probes
                .receive(token + 2, Value::Float(-0.02), context())
                .is_none()
        );
        assert!(
            probes
                .receive(token, Value::Integer(48_000), context())
                .is_none()
        );
        assert!(
            probes
                .receive(token, Value::Integer(1), context())
                .is_none()
        );
        let reply = probes
            .receive(token + 1, Value::Integer(96_000), context())
            .unwrap();
        assert_eq!(reply.decoder_sample_rate, Some(48_000));
        assert_eq!(reply.output_sample_rate, Some(96_000));
        assert_eq!(reply.audio_pts, Some(-0.02));
        assert!(
            probes
                .receive(token + 1, Value::Integer(1), context())
                .is_none()
        );
        assert_ne!(probes.begin(context()).unwrap(), token);
    }
    #[test]
    fn cancellation_keeps_native_admission_reserved_until_all_replies_are_reaped() {
        let mut probes = AudioProbes::default();
        let token = submit(&mut probes);
        probes.cancel(token + 4); // unrelated token cannot cancel current work
        probes.cancel(token);
        for field in 0..3 {
            assert!(probes.begin(context()).is_err());
            assert!(
                probes
                    .receive(token + field, Value::Unavailable, context())
                    .is_none()
            );
        }
        assert!(probes.begin(context()).is_ok());
    }
    #[test]
    fn stale_generation_load_or_native_entry_never_publishes() {
        for stale in [
            Context {
                generation: 4,
                ..context()
            },
            Context {
                load_request_id: 43,
                ..context()
            },
            Context {
                entry: Some(8),
                ..context()
            },
            Context {
                entry: None,
                ..context()
            },
        ] {
            let mut probes = AudioProbes::default();
            let token = submit(&mut probes);
            assert!(
                probes
                    .receive(token + 9, Value::Integer(1), stale)
                    .is_none()
            );
            for field in 0..3 {
                assert!(
                    probes
                        .receive(token + field, Value::Integer(48_000), stale)
                        .is_none()
                );
            }
            assert!(probes.begin(context()).is_ok());
        }
    }
    #[test]
    fn partial_submission_failure_holds_only_the_accepted_native_replies() {
        let mut probes = AudioProbes::default();
        let token = probes.begin(context()).unwrap();
        probes.submitted(0);
        probes.abort_submission();
        assert!(probes.begin(context()).is_err());
        assert!(
            probes
                .receive(token, Value::Integer(48_000), context())
                .is_none()
        );
        probes.begin(context()).unwrap();
        probes.abort_submission();
        assert!(probes.begin(context()).is_ok());
    }
    #[test]
    fn invalid_values_remain_unknown_and_token_namespace_cannot_wrap() {
        let mut probes = AudioProbes::default();
        let token = submit(&mut probes);
        probes.receive(token, Value::Integer(0), context());
        probes.receive(token + 1, Value::Unavailable, context());
        let reply = probes
            .receive(token + 2, Value::Float(f64::NAN), context())
            .unwrap();
        assert_eq!(
            (
                reply.decoder_sample_rate,
                reply.output_sample_rate,
                reply.audio_pts
            ),
            (None, None, None)
        );
        assert!(owns_userdata(token));
        assert!(!owns_userdata(1 << 62));
        assert!(!owns_userdata(20));
        probes.next_token = LIMIT - 4;
        assert!(probes.begin(context()).is_err());
    }
}
