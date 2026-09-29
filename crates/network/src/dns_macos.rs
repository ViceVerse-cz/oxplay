//! macOS system DNS without parsing/filtering scoped resolver configuration.
//!
//! FFI checked against the installed macOS 27 SDK `dns_sd.h`: callbacks run
//! synchronously inside DNSServiceProcessResult when no dispatch queue is set.
//! Only readiness is observed directly; the DNS-SD API owns all socket I/O.
use crate::policy;
use std::{
    ffi::{CString, c_char, c_int, c_void},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::fd::{AsRawFd, RawFd},
    ptr,
    time::Duration,
};
use tokio::io::unix::AsyncFd;

const IPV4: u32 = 0x01;
const IPV6: u32 = 0x02;
const MORE_COMING: u32 = 0x01;
const ADD: u32 = 0x02;
const NO_SUCH_RECORD: i32 = -65554;
const MAX_ADDRESSES_PER_FAMILY: usize = 32;
const DEADLINE: Duration = Duration::from_secs(3);
type ServiceRef = *mut c_void;
type Callback = unsafe extern "C" fn(
    ServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const libc::sockaddr,
    u32,
    *mut c_void,
);

#[link(name = "System")]
unsafe extern "C" {
    fn DNSServiceGetAddrInfo(
        reference: *mut ServiceRef,
        flags: u32,
        interface: u32,
        protocol: u32,
        hostname: *const c_char,
        callback: Callback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(reference: ServiceRef) -> c_int;
    fn DNSServiceProcessResult(reference: ServiceRef) -> i32;
    fn DNSServiceRefDeallocate(reference: ServiceRef);
}

pub(crate) async fn resolve_host(hostname: &str) -> io::Result<Vec<SocketAddr>> {
    if hostname.is_empty()
        || hostname.len() > 253
        || !hostname
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(failure("Media DNS name is invalid"));
    }
    // Absolute names prevent search-domain expansion. Interface 0 delegates
    // actual scoped resolver selection to the native system daemon.
    let hostname = CString::new(format!("{}.", hostname.trim_end_matches('.')))
        .map_err(|_| failure("Media DNS name is invalid"))?;
    tokio::time::timeout(DEADLINE, async {
        let ipv4 = Query::start(&hostname, IPV4)?;
        let ipv6 = Query::start(&hostname, IPV6)?;
        let (mut addresses, other) = tokio::try_join!(ipv4.finish(), ipv6.finish())?;
        addresses.extend(other);
        if addresses.is_empty() {
            return Err(failure("Media DNS returned no public addresses"));
        }
        Ok(addresses)
    })
    .await
    .map_err(|_| failure("Media DNS lookup timed out"))?
}

fn failure(message: &'static str) -> io::Error {
    io::Error::other(message)
}

struct NativeService(ServiceRef);
// A reference is not concurrently thread-safe. This uniquely owned value can
// move between executor threads: no dispatch queue is installed and every API
// call/callback is synchronous and serialized by its owning future. Not Sync.
unsafe impl Send for NativeService {}
impl Drop for NativeService {
    fn drop(&mut self) {
        // SAFETY: only successfully initialized, uniquely owned references live
        // here. Query drops readiness registration first and callback state last.
        unsafe { DNSServiceRefDeallocate(self.0) };
    }
}
struct DnsFd(RawFd);
impl AsRawFd for DnsFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
// No Drop for DnsFd: DNSServiceRefDeallocate alone owns/closes the descriptor.

#[derive(Default)]
struct ReplyState {
    addresses: Vec<SocketAddr>,
    done: bool,
    rejected: bool,
}
struct Query {
    // Field drop order is intentional and required by dns_sd.h. A timeout or
    // dropped reqwest future deregisters kqueue before closing the native fd,
    // then frees the stable callback allocation after callbacks are impossible.
    readiness: AsyncFd<DnsFd>,
    service: NativeService,
    state: Box<ReplyState>,
}
impl Query {
    fn start(hostname: &CString, protocol: u32) -> io::Result<Self> {
        let mut state = Box::<ReplyState>::default();
        let mut reference = ptr::null_mut();
        // SAFETY: CString and the stable heap state outlive initialization. The
        // SDK copies hostname; subsequent callbacks retain only our state ptr.
        let result = unsafe {
            DNSServiceGetAddrInfo(
                &mut reference,
                0,
                0,
                protocol,
                hostname.as_ptr(),
                reply,
                (&mut *state as *mut ReplyState).cast(),
            )
        };
        if result != 0 || reference.is_null() {
            // On failure the SDK says the output reference is not initialized.
            return Err(failure("Native media DNS initialization failed"));
        }
        let service = NativeService(reference);
        // SAFETY: a successful API call initialized this reference.
        let fd = unsafe { DNSServiceRefSockFD(reference) };
        if fd < 0 {
            return Err(failure("Native media DNS descriptor is unavailable"));
        }
        // AsyncFd requires O_NONBLOCK. The native client handles empty-header
        // EWOULDBLOCK; a partial native IPC frame may instead fail processing.
        // Such errors fail closed, never fall back to a different DNS server.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(failure("Native media DNS readiness setup failed"));
        }
        let readiness = AsyncFd::new(DnsFd(fd))?;
        Ok(Self {
            readiness,
            service,
            state,
        })
    }

    async fn finish(mut self) -> io::Result<Vec<SocketAddr>> {
        loop {
            // The native callback mutates this stable state allocation during
            // ProcessResult; the mutation is deliberately outside Rust syntax.
            if self.state.done {
                break;
            }
            let mut ready = self.readiness.readable().await?;
            // SAFETY: readiness was signalled; this future is the sole caller.
            // O_NONBLOCK also protects against spurious reactor readiness.
            let result = unsafe { DNSServiceProcessResult(self.service.0) };
            ready.clear_ready();
            if result != 0 {
                return Err(failure("Native media DNS processing failed"));
            }
        }
        if self.state.rejected {
            return Err(failure("Media DNS policy rejected the answer"));
        }
        Ok(std::mem::take(&mut self.state.addresses))
    }
}

unsafe extern "C" fn reply(
    _reference: ServiceRef,
    flags: u32,
    _interface: u32,
    error: i32,
    _hostname: *const c_char,
    address: *const libc::sockaddr,
    _ttl: u32,
    context: *mut c_void,
) {
    // SAFETY: the stable Box is owned until after service deallocation; native
    // callbacks are synchronous during the sole owner's ProcessResult call.
    let state = unsafe { &mut *context.cast::<ReplyState>() };
    if state.done {
        return;
    }
    if error != 0 {
        // Address/flags/hostname are undefined on native errors. In particular,
        // no AAAA record must not discard a successful independent A lookup.
        state.rejected = error != NO_SUCH_RECORD;
        state.done = true;
        return;
    }
    if address.is_null() {
        state.rejected = true;
        state.done = true;
        return;
    }
    // SAFETY: successful callbacks provide a valid native sockaddr; length is
    // checked before reading a larger family-specific representation.
    let header = unsafe { &*address };
    let value = match c_int::from(header.sa_family) {
        libc::AF_INET if usize::from(header.sa_len) >= size_of::<libc::sockaddr_in>() => {
            let native = unsafe { ptr::read_unaligned(address.cast::<libc::sockaddr_in>()) };
            SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(native.sin_addr.s_addr.to_ne_bytes())),
                0,
            )
        }
        libc::AF_INET6 if usize::from(header.sa_len) >= size_of::<libc::sockaddr_in6>() => {
            let native = unsafe { ptr::read_unaligned(address.cast::<libc::sockaddr_in6>()) };
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(native.sin6_addr.s6_addr),
                0,
                0,
                native.sin6_scope_id,
            ))
        }
        _ => {
            state.rejected = true;
            state.done = true;
            return;
        }
    };
    if !policy::public_ip(value.ip()) {
        state.rejected = true;
        state.done = true;
        return;
    }
    if flags & ADD != 0 {
        if !state.addresses.contains(&value) {
            if state.addresses.len() >= MAX_ADDRESSES_PER_FAMILY {
                state.rejected = true;
                state.done = true;
                return;
            }
            state.addresses.push(value);
        }
    } else {
        state.addresses.retain(|existing| *existing != value);
    }
    state.done = flags & MORE_COMING == 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv4_reply(state: &mut ReplyState, ip: [u8; 4], flags: u32) {
        let address = libc::sockaddr_in {
            sin_len: size_of::<libc::sockaddr_in>() as u8,
            sin_family: libc::AF_INET as u8,
            sin_port: 0,
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes(ip),
            },
            sin_zero: [0; 8],
        };
        unsafe {
            reply(
                ptr::null_mut(),
                flags,
                0,
                0,
                ptr::null(),
                (&address as *const libc::sockaddr_in).cast(),
                0,
                (state as *mut ReplyState).cast(),
            )
        };
    }

    #[test]
    fn callbacks_deduplicate_and_reject_mixed_public_private_answers() {
        let mut state = ReplyState::default();
        ipv4_reply(&mut state, [8, 8, 8, 8], ADD | MORE_COMING);
        ipv4_reply(&mut state, [8, 8, 8, 8], ADD | MORE_COMING);
        assert_eq!(state.addresses.len(), 1);
        ipv4_reply(&mut state, [127, 0, 0, 1], ADD);
        assert!(state.done && state.rejected);
    }

    #[test]
    fn absent_family_ignores_undefined_callback_fields_without_rejecting_other_family() {
        let mut state = ReplyState::default();
        unsafe {
            reply(
                ptr::null_mut(),
                u32::MAX,
                u32::MAX,
                NO_SUCH_RECORD,
                ptr::null(),
                ptr::null(),
                u32::MAX,
                (&mut state as *mut ReplyState).cast(),
            )
        };
        assert!(state.done && !state.rejected);
        assert!(state.addresses.is_empty());
    }

    #[test]
    fn native_reply_count_is_bounded() {
        let mut state = ReplyState::default();
        for n in 1..=MAX_ADDRESSES_PER_FAMILY + 1 {
            ipv4_reply(&mut state, [8, 8, 8, n as u8], ADD | MORE_COMING);
        }
        assert!(state.rejected && state.done);
        assert_eq!(state.addresses.len(), MAX_ADDRESSES_PER_FAMILY);
    }

    #[test]
    fn malformed_address_length_fails_closed() {
        let mut state = ReplyState::default();
        let address = libc::sockaddr {
            sa_len: 0,
            sa_family: libc::AF_INET6 as u8,
            sa_data: [0; 14],
        };
        unsafe {
            reply(
                ptr::null_mut(),
                ADD,
                0,
                0,
                ptr::null(),
                &address,
                0,
                (&mut state as *mut ReplyState).cast(),
            )
        };
        assert!(state.rejected && state.done);
    }

    #[test]
    // Primitive-only check: production calls this implementation inside the
    // supervised serein-dns process, not directly from the application runtime.
    #[ignore = "Explicit DNS-SD primitive smoke: resolves example.com in-process; no HTTP/account; production helper checked separately"]
    fn native_resolution_and_query_drop_close_the_owned_socket() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let addresses = resolve_host("example.com").await.unwrap();
            assert!(!addresses.is_empty());
            assert!(
                addresses
                    .iter()
                    .all(|address| policy::public_ip(address.ip()))
            );
            let name = CString::new("example.com.").unwrap();
            let query = Query::start(&name, IPV4).unwrap();
            let fd = query.readiness.as_raw_fd();
            assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0);
            drop(query);
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
        });
    }
}
