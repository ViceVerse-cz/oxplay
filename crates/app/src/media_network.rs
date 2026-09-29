// SPDX-License-Identifier: GPL-3.0-or-later
//! The only bridge between the HTTP policy layer and the media callback ABI.
use serein_core::ResolvedPlayback;
use serein_media::{
    Player,
    streams::{OpenedStream, StreamCancel, StreamError, StreamFactory, StreamReader, StreamResult},
};
use serein_network::{AccessLease, Cancellation, HttpReader, HttpSource, NetworkConfig};
use serein_youtube::account::AccountPlaybackLease;
use std::{future::Future, pin::Pin, sync::Arc};

struct Factory(HttpSource);
struct Reader(HttpReader);
struct Cancel(Arc<Cancellation>);
/// No credentials or addresses cross this adapter: only revocable authority.
struct AccountAccess(AccountPlaybackLease);
impl AccessLease for AccountAccess {
    fn is_valid(&self) -> bool {
        self.0.is_valid()
    }
    fn revoked(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.0.revoked())
    }
}
fn error(value: serein_network::Error) -> StreamError {
    match value {
        serein_network::Error::Cancelled | serein_network::Error::AccessRevoked => {
            StreamError::Cancelled
        }
        serein_network::Error::Policy | serein_network::Error::InvalidResponse => {
            StreamError::InvalidData
        }
        serein_network::Error::Unsupported | serein_network::Error::HttpStatus(_) => {
            StreamError::Unsupported
        }
        serein_network::Error::Timeout
        | serein_network::Error::Transport
        | serein_network::Error::DnsConfiguration => StreamError::Transport,
    }
}
impl StreamCancel for Cancel {
    fn cancel(&self) {
        self.0.cancel();
    }
}
impl StreamReader for Reader {
    fn read(&mut self, destination: &mut [u8]) -> StreamResult<usize> {
        self.0.read(destination).map_err(error)
    }
    fn seek(&mut self, absolute: u64) -> StreamResult<u64> {
        self.0.seek(absolute).map_err(error)
    }
    fn size(&mut self) -> StreamResult<Option<u64>> {
        self.0.size().map(Some).map_err(error)
    }
}
impl StreamFactory for Factory {
    fn open(&self) -> StreamResult<OpenedStream> {
        let (reader, cancel) = self.0.open_checked().map_err(error)?;
        Ok(OpenedStream {
            reader: Box::new(Reader(reader)),
            cancel: Arc::new(Cancel(cancel)),
        })
    }
}
pub fn load(
    player: &Player,
    playback: &ResolvedPlayback,
    config: &NetworkConfig,
    position: f64,
    paused: bool,
) -> Result<(), String> {
    if !playback.guest || playback.session_generation != 0 {
        return Err("Guest media requires an anonymous playback result.".into());
    }
    load_sources(player, playback, config, None, position, paused)
}

/// Explicit account-only entry point. Generation matching is checked before
/// constructing either source. The UI must also cancel session work and stop
/// the player on sign-out: a lease cannot recall bytes already delivered.
pub fn load_with_authorization(
    player: &Player,
    playback: &ResolvedPlayback,
    authorization: &AccountPlaybackLease,
    config: &NetworkConfig,
    position: f64,
    paused: bool,
) -> Result<(), String> {
    if playback.guest
        || playback.session_generation != authorization.generation()
        || !authorization.is_valid()
    {
        return Err(
            "Account media authorization is unavailable or belongs to another session.".into(),
        );
    }
    let access: Arc<dyn AccessLease> = Arc::new(AccountAccess(authorization.clone()));
    load_sources(player, playback, config, Some(access), position, paused)
}

fn load_sources(
    player: &Player,
    playback: &ResolvedPlayback,
    config: &NetworkConfig,
    access: Option<Arc<dyn AccessLease>>,
    position: f64,
    paused: bool,
) -> Result<(), String> {
    let factory = |track| -> Result<Arc<dyn StreamFactory>, String> {
        let source = match &access {
            Some(lease) => HttpSource::authorized(track, config, lease.clone()),
            None => HttpSource::guest(track, config),
        }
        .map_err(|e| e.to_string())?;
        Ok(Arc::new(Factory(source)))
    };
    let video = factory(&playback.video_track)?;
    let audio = playback.audio_track.as_ref().map(factory).transpose()?;
    if access.as_ref().is_some_and(|lease| !lease.is_valid()) {
        return Err("Account media authorization was revoked before playback could start.".into());
    }
    player
        .load_streams_at(video, audio, position, paused)
        .map_err(|e| e.to_string())
}
