// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded compressed-byte callbacks, audited against mpv 0.41 stream_cb.h/.c.
//! No callbacks may call this mpv instance. Factories and initial seek(0) must
//! perform no I/O: mpv registers cancellation only after those operations.
use crate::{MediaError, Result, checked, ffi};
use std::{
    collections::BTreeMap,
    ffi::{CStr, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    Cancelled,
    Unsupported,
    InvalidData,
    Transport,
}
pub type StreamResult<T> = std::result::Result<T, StreamError>;
/// Reads may block the demux thread, never the UI. Zero means final EOF.
/// Implementations must impose deadlines and bound their own compressed buffers.
pub trait StreamReader: Send {
    fn read(&mut self, buffer: &mut [u8]) -> StreamResult<usize>;
    /// Absolute seek; initial seek(0) must not perform I/O.
    fn seek(&mut self, absolute: u64) -> StreamResult<u64>;
    fn size(&mut self) -> StreamResult<Option<u64>>;
}
/// Must be nonblocking, idempotent, and interrupt present and future operations.
pub trait StreamCancel: Send + Sync {
    fn cancel(&self);
}
pub struct OpenedStream {
    pub reader: Box<dyn StreamReader>,
    pub cancel: Arc<dyn StreamCancel>,
}
/// Opening constructs lazy state only, without DNS, network or filesystem I/O.
pub trait StreamFactory: Send + Sync {
    fn open(&self) -> StreamResult<OpenedStream>;
}

const MAX_STREAMS: usize = 4;
const MAX_READ: usize = 64 * 1024;
const PREFIX: &str = "serein-stream://";
#[derive(Default)]
struct State {
    next: u64,
    sources: BTreeMap<u64, Arc<dyn StreamFactory>>,
    live: Vec<Weak<Session>>,
}
struct Session {
    id: u64,
    cancelled: AtomicBool,
    cancel: Arc<dyn StreamCancel>,
}
impl Session {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            let _ = catch_unwind(AssertUnwindSafe(|| self.cancel.cancel()));
        }
    }
}
#[derive(Default)]
pub(crate) struct Registry {
    state: Mutex<State>,
    active: Arc<AtomicUsize>,
}
pub(crate) struct Pair {
    pub video: String,
    pub audio: Option<String>,
    ids: Vec<u64>,
}
impl Registry {
    /// Registry has stable Box storage retained until terminate_destroy returns.
    pub unsafe fn install(&self, raw: *mut ffi::Handle) -> Result<()> {
        unsafe {
            checked(
                mpv_stream_cb_add_ro(
                    raw,
                    c"serein-stream".as_ptr(),
                    std::ptr::from_ref(self).cast_mut().cast(),
                    Some(open),
                ),
                "Register compressed media transport",
            )
        }
    }
    pub fn prepare(
        &self,
        video: Arc<dyn StreamFactory>,
        audio: Option<Arc<dyn StreamFactory>>,
    ) -> Result<Pair> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MediaError("Media stream registry unavailable".into()))?;
        let count = 1 + usize::from(audio.is_some());
        if state.sources.len() + count > MAX_STREAMS {
            return Err(MediaError("Media stream registration limit reached".into()));
        }
        state
            .next
            .checked_add(count as u64)
            .ok_or_else(|| MediaError("Media stream identifiers exhausted".into()))?;
        let mut ids = Vec::with_capacity(count);
        for source in std::iter::once(video).chain(audio) {
            state.next = state
                .next
                .checked_add(1)
                .ok_or_else(|| MediaError("Media stream identifiers exhausted".into()))?;
            let id = state.next;
            state.sources.insert(id, source);
            ids.push(id);
        }
        Ok(Pair {
            video: format!("{PREFIX}{}", ids[0]),
            audio: ids.get(1).map(|id| format!("{PREFIX}{id}")),
            ids,
        })
    }
    pub fn commit(&self, pair: &Pair) {
        self.remove_where(|id| !pair.ids.contains(&id));
    }
    pub fn rollback(&self, pair: &Pair) {
        self.remove_where(|id| pair.ids.contains(&id));
    }
    pub fn clear(&self) {
        self.remove_where(|_| true);
    }
    fn remove_where(&self, remove: impl Fn(u64) -> bool) {
        let cancelled = if let Ok(mut state) = self.state.lock() {
            state.sources.retain(|id, _| !remove(*id));
            state.live.retain(|s| s.strong_count() > 0);
            state
                .live
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|s| remove(s.id))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // Cancellation never runs under registry/reader locks.
        for session in cancelled {
            session.cancel();
        }
    }
    fn open(&self, uri: &str) -> StreamResult<Cookie> {
        let id = uri
            .strip_prefix(PREFIX)
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or(StreamError::InvalidData)?;
        let factory = self
            .state
            .lock()
            .map_err(|_| StreamError::Transport)?
            .sources
            .get(&id)
            .cloned()
            .ok_or(StreamError::Cancelled)?;
        let active = Active::reserve(self.active.clone())?;
        let stream = factory.open()?;
        let session = Arc::new(Session {
            id,
            cancelled: AtomicBool::new(false),
            cancel: stream.cancel,
        });
        {
            let mut state = self.state.lock().map_err(|_| StreamError::Transport)?;
            if !state.sources.contains_key(&id) {
                drop(state);
                session.cancel();
                return Err(StreamError::Cancelled);
            }
            state.live.retain(|s| s.strong_count() > 0);
            state.live.push(Arc::downgrade(&session));
        }
        Ok(Cookie {
            reader: Mutex::new(stream.reader),
            session,
            _active: active,
        })
    }
}
struct Active(Arc<AtomicUsize>);
impl Active {
    fn reserve(counter: Arc<AtomicUsize>) -> StreamResult<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                (v < MAX_STREAMS).then_some(v + 1)
            })
            .map_err(|_| StreamError::Transport)?;
        Ok(Self(counter))
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Cookie {
    reader: Mutex<Box<dyn StreamReader>>,
    session: Arc<Session>,
    _active: Active,
}
impl Drop for Cookie {
    fn drop(&mut self) {
        self.session.cancel();
    }
}
impl Cookie {
    fn with<T>(&self, f: impl FnOnce(&mut dyn StreamReader) -> StreamResult<T>) -> StreamResult<T> {
        if self.session.cancelled.load(Ordering::Acquire) {
            return Err(StreamError::Cancelled);
        }
        let mut reader = self.reader.lock().map_err(|_| StreamError::Transport)?;
        if self.session.cancelled.load(Ordering::Acquire) {
            return Err(StreamError::Cancelled);
        }
        f(reader.as_mut())
    }
}
#[repr(C)]
struct Info {
    cookie: *mut c_void,
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void)>,
}
unsafe extern "C" {
    fn mpv_stream_cb_add_ro(
        raw: *mut ffi::Handle,
        protocol: *const c_char,
        data: *mut c_void,
        open: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, *mut Info) -> i32>,
    ) -> i32;
}
unsafe extern "C" fn open(data: *mut c_void, uri: *mut c_char, info: *mut Info) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if data.is_null() || uri.is_null() || info.is_null() {
            return -13;
        }
        let registry = unsafe { &*data.cast::<Registry>() };
        let Ok(uri) = unsafe { CStr::from_ptr(uri) }.to_str() else {
            return -13;
        };
        let Ok(cookie) = registry.open(uri) else {
            return -13;
        };
        unsafe {
            info.write(Info {
                cookie: Box::into_raw(Box::new(cookie)).cast(),
                read: Some(read),
                seek: Some(seek),
                size: Some(size),
                close: Some(close),
                cancel: Some(cancel),
            });
        }
        0
    }))
    .unwrap_or(-13)
}
unsafe extern "C" fn read(data: *mut c_void, buffer: *mut c_char, len: u64) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        if data.is_null() || (buffer.is_null() && len > 0) {
            return -1;
        }
        if len == 0 {
            return 0;
        }
        let count = len.min(MAX_READ as u64) as usize;
        let bytes = unsafe { std::slice::from_raw_parts_mut(buffer.cast::<u8>(), count) };
        let cookie = unsafe { &*data.cast::<Cookie>() };
        match cookie.with(|r| r.read(bytes)) {
            Ok(n) if n <= count => n as i64,
            _ => -1,
        }
    }))
    .unwrap_or(-1)
}
unsafe extern "C" fn seek(data: *mut c_void, position: i64) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        if data.is_null() || position < 0 {
            return -20;
        }
        let cookie = unsafe { &*data.cast::<Cookie>() };
        match cookie.with(|r| r.seek(position as u64)) {
            Ok(n) if n == position as u64 => position,
            Err(StreamError::Unsupported) => -18,
            _ => -20,
        }
    }))
    .unwrap_or(-20)
}
unsafe extern "C" fn size(data: *mut c_void) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        if data.is_null() {
            return -20;
        }
        let cookie = unsafe { &*data.cast::<Cookie>() };
        match cookie.with(|r| r.size()) {
            Ok(Some(n)) if n <= i64::MAX as u64 => n as i64,
            Ok(None) | Err(StreamError::Unsupported) => -18,
            _ => -20,
        }
    }))
    .unwrap_or(-20)
}
unsafe extern "C" fn cancel(data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !data.is_null() {
            unsafe { &*data.cast::<Cookie>() }.session.cancel();
        }
    }));
}
unsafe extern "C" fn close(data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !data.is_null() {
            drop(unsafe { Box::from_raw(data.cast::<Cookie>()) });
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Cursor, Read, Seek, SeekFrom},
        time::{Duration, Instant},
    };
    #[derive(Default)]
    struct Flag(AtomicBool);
    impl StreamCancel for Flag {
        fn cancel(&self) {
            self.0.store(true, Ordering::Release);
        }
    }
    struct Memory {
        data: Cursor<Vec<u8>>,
        cancel: Arc<Flag>,
    }
    impl StreamReader for Memory {
        fn read(&mut self, b: &mut [u8]) -> StreamResult<usize> {
            if self.cancel.0.load(Ordering::Acquire) {
                return Err(StreamError::Cancelled);
            }
            self.data.read(b).map_err(|_| StreamError::Transport)
        }
        fn seek(&mut self, p: u64) -> StreamResult<u64> {
            self.data
                .seek(SeekFrom::Start(p))
                .map_err(|_| StreamError::Transport)
        }
        fn size(&mut self) -> StreamResult<Option<u64>> {
            Ok(Some(self.data.get_ref().len() as u64))
        }
    }
    struct Factory(Vec<u8>);
    impl StreamFactory for Factory {
        fn open(&self) -> StreamResult<OpenedStream> {
            let cancel = Arc::new(Flag::default());
            Ok(OpenedStream {
                reader: Box::new(Memory {
                    data: Cursor::new(self.0.clone()),
                    cancel: cancel.clone(),
                }),
                cancel,
            })
        }
    }
    #[test]
    fn callbacks_bound_reads_seek_and_invalidate_replaced_streams() {
        let registry = Registry::default();
        let pair = registry
            .prepare(Arc::new(Factory(vec![7; MAX_READ * 2])), None)
            .unwrap();
        let mut cookie = registry.open(&pair.video).unwrap();
        let pointer = std::ptr::from_mut(&mut cookie).cast();
        let mut buffer = vec![0u8; MAX_READ * 2];
        assert_eq!(
            unsafe { read(pointer, buffer.as_mut_ptr().cast(), buffer.len() as u64) },
            MAX_READ as i64
        );
        assert!(buffer[..MAX_READ].iter().all(|b| *b == 7));
        assert!(buffer[MAX_READ..].iter().all(|b| *b == 0));
        assert_eq!(unsafe { size(pointer) }, (MAX_READ * 2) as i64);
        assert_eq!(unsafe { seek(pointer, 0) }, 0);
        assert_eq!(unsafe { seek(pointer, -1) }, -20);
        let newest = registry.prepare(Arc::new(Factory(vec![1])), None).unwrap();
        registry.commit(&newest);
        assert!(matches!(
            registry.open(&pair.video),
            Err(StreamError::Cancelled)
        ));
        assert_eq!(unsafe { read(pointer, buffer.as_mut_ptr().cast(), 1) }, -1);
        let a = registry.open(&newest.video).unwrap();
        let b = registry.open(&newest.video).unwrap();
        let c = registry.open(&newest.video).unwrap();
        assert!(matches!(
            registry.open(&newest.video),
            Err(StreamError::Transport)
        ));
        drop((cookie, a, b, c));
        assert_eq!(registry.active.load(Ordering::Acquire), 0);
    }
    struct Blocking(std::sync::mpsc::Receiver<()>);
    impl StreamReader for Blocking {
        fn read(&mut self, _: &mut [u8]) -> StreamResult<usize> {
            let _ = self.0.recv_timeout(Duration::from_secs(2));
            Err(StreamError::Cancelled)
        }
        fn seek(&mut self, p: u64) -> StreamResult<u64> {
            Ok(p)
        }
        fn size(&mut self) -> StreamResult<Option<u64>> {
            Ok(None)
        }
    }
    struct Interrupt(std::sync::mpsc::SyncSender<()>);
    impl StreamCancel for Interrupt {
        fn cancel(&self) {
            let _ = self.0.try_send(());
        }
    }
    #[test]
    fn cancellation_does_not_wait_for_blocked_reader_mutex() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let counter = Arc::new(AtomicUsize::new(0));
        let cookie = Arc::new(Cookie {
            reader: Mutex::new(Box::new(Blocking(rx))),
            session: Arc::new(Session {
                id: 1,
                cancelled: AtomicBool::new(false),
                cancel: Arc::new(Interrupt(tx)),
            }),
            _active: Active::reserve(counter).unwrap(),
        });
        let cloned = cookie.clone();
        let thread = std::thread::spawn(move || cloned.with(|r| r.read(&mut [0; 1])));
        let deadline = Instant::now() + Duration::from_secs(1);
        while cookie.reader.try_lock().is_ok() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let start = Instant::now();
        unsafe {
            cancel(Arc::as_ptr(&cookie).cast_mut().cast());
        }
        assert_eq!(thread.join().unwrap(), Err(StreamError::Cancelled));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(cookie.session.cancelled.load(Ordering::Acquire));
    }
    struct Engine {
        raw: *mut ffi::Handle,
        _registry: Box<Registry>,
    }
    impl Drop for Engine {
        fn drop(&mut self) {
            self._registry.clear();
            unsafe {
                ffi::mpv_terminate_destroy(self.raw);
            }
        }
    }
    #[test]
    fn native_demux_reads_custom_stream_and_teardown_closes_all_cookies() {
        let mut bytes = b"RIFF".to_vec();
        let samples = 24_000u32;
        bytes.extend((36 + samples * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48_000u32.to_le_bytes());
        bytes.extend(96_000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples * 2).to_le_bytes());
        bytes.resize(44 + samples as usize * 2, 0);
        let raw = unsafe { ffi::mpv_create() };
        assert!(!raw.is_null());
        let engine = Engine {
            raw,
            _registry: Box::default(),
        };
        for (key, value) in [
            (c"config", c"no"),
            (c"terminal", c"no"),
            (c"load-scripts", c"no"),
            (c"ytdl", c"no"),
            (c"vo", c"null"),
            (c"ao", c"null"),
            (c"pause", c"yes"),
            (c"idle", c"yes"),
            (c"access-references", c"no"),
        ] {
            assert!(unsafe { ffi::mpv_set_option_string(raw, key.as_ptr(), value.as_ptr()) } >= 0);
        }
        assert!(unsafe { ffi::mpv_initialize(raw) } >= 0);
        unsafe {
            engine._registry.install(raw).unwrap();
        }
        let pair = engine
            ._registry
            .prepare(Arc::new(Factory(bytes)), None)
            .unwrap();
        unsafe {
            crate::commands::loadfile(raw, 1, &pair.video, "0", None, None).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let event = unsafe { &*ffi::mpv_wait_event(raw, 0.05) };
            if event.id == 8 {
                break;
            }
            assert!(event.id != 7, "native stream ended before loading");
            assert!(
                Instant::now() < deadline,
                "native custom stream load timed out"
            );
        }
        let active = engine._registry.active.clone();
        assert!(active.load(Ordering::Acquire) > 0);
        drop(engine);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
}
