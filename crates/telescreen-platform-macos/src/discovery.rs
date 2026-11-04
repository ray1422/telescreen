//! Discovery with Bonjour (NET-12): `dns_sd` of libSystem.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Write;
use std::net::ToSocketAddrs;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use telescreen_core::platform::{
    DISCOVERY_SERVICE, Discovery, DiscoveryEvent, DiscoverySink, FoundPeer, PlatformError,
    discovery_instance_name, fingerprint_of_txt,
};

type ServiceRef = *mut c_void;
type RegisterReply =
    extern "C" fn(ServiceRef, u32, i32, *const c_char, *const c_char, *const c_char, *mut c_void);
type BrowseReply = extern "C" fn(
    ServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
);
type ResolveReply = extern "C" fn(
    ServiceRef,
    u32,
    u32,
    i32,
    *const c_char,
    *const c_char,
    u16,
    u16,
    *const u8,
    *mut c_void,
);

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

#[link(name = "System")]
unsafe extern "C" {
    fn DNSServiceRegister(
        service: *mut ServiceRef,
        flags: u32,
        interface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        txt_len: u16,
        txt: *const c_void,
        callback: RegisterReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceBrowse(
        service: *mut ServiceRef,
        flags: u32,
        interface: u32,
        regtype: *const c_char,
        domain: *const c_char,
        callback: BrowseReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceResolve(
        service: *mut ServiceRef,
        flags: u32,
        interface: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        callback: ResolveReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(service: ServiceRef) -> c_int;
    fn DNSServiceProcessResult(service: ServiceRef) -> i32;
    fn DNSServiceRefDeallocate(service: ServiceRef);
    fn poll(fds: *mut PollFd, count: u32, timeout: c_int) -> c_int;
}

struct Service(ServiceRef);

// SAFETY: a service moves to one worker, which alone processes and deallocates it.
unsafe impl Send for Service {}

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the reference, and no callback is running.
        unsafe { DNSServiceRefDeallocate(self.0) };
    }
}

struct Worker {
    wake: UnixStream,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.wake.write_all(&[1]);
        let _ = self.thread.join();
    }
}

/// The Bonjour backend.
pub struct MacDiscovery {
    sink: DiscoverySink,
    announce: Option<Worker>,
    search: Option<Worker>,
}

impl std::fmt::Debug for MacDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacDiscovery").finish_non_exhaustive()
    }
}

impl MacDiscovery {
    pub fn new(sink: DiscoverySink) -> Self {
        Self {
            sink,
            announce: None,
            search: None,
        }
    }
}

fn dns_error(code: i32) -> PlatformError {
    if code == -65570 {
        PlatformError::Permission(format!(
            "Bonjour: missing Local Network permission ({code})"
        ))
    } else {
        PlatformError::Failed(format!("Bonjour error {code}"))
    }
}

fn io_error(error: std::io::Error) -> PlatformError {
    PlatformError::Failed(format!(
        "Bonjour: {error} (code {})",
        error.raw_os_error().unwrap_or(-1)
    ))
}

fn txt_record(fingerprint: &str) -> Result<Vec<u8>, PlatformError> {
    let entry = format!("fp={fingerprint}");
    let length = u8::try_from(entry.len()).map_err(|_| {
        PlatformError::Failed("Bonjour TXT entry exceeds 255 bytes (-65540)".into())
    })?;
    let mut txt = vec![length];
    txt.extend_from_slice(entry.as_bytes());
    Ok(txt)
}

fn txt_fingerprint(mut txt: &[u8]) -> Option<String> {
    while let Some((&length, rest)) = txt.split_first() {
        let length = usize::from(length);
        let entry = rest.get(..length)?;
        if let Some(fingerprint) = fingerprint_of_txt(entry) {
            return Some(fingerprint);
        }
        txt = &rest[length..];
    }
    None
}

struct Record {
    interface: u32,
    name: CString,
    service: CString,
    domain: CString,
}

struct Resolved {
    host: String,
    port: u16,
    fingerprint: String,
}

#[derive(Default)]
struct Replies {
    error: Option<i32>,
    records: Vec<Record>,
    resolved: Option<Resolved>,
}

/// The callback context of one service. It lives in a `Box`, and a callback gets
/// its address.
type Context = Mutex<Replies>;

fn context_pointer(context: &Context) -> *mut c_void {
    ptr::from_ref(context).cast_mut().cast()
}

// Each callback only copies its reply. The worker does all network work and calls
// the sink after DNSServiceProcessResult returns. No panic crosses the C ABI.
fn reply(context: *mut c_void, error: i32, action: impl FnOnce(&mut Replies)) {
    if context.is_null() {
        return;
    }
    // SAFETY: the caller keeps this boxed context alive through service teardown.
    // It is a shared reference: the worker reads the same context between callbacks.
    let replies = unsafe { &*context.cast_const().cast::<Context>() };
    let mut replies = replies.lock().unwrap_or_else(PoisonError::into_inner);
    if error != 0 {
        replies.error = Some(error);
    } else if catch_unwind(AssertUnwindSafe(|| action(&mut replies))).is_err() {
        replies.error = Some(-65537);
    }
}

extern "C" fn registered(
    _: ServiceRef,
    _: u32,
    error: i32,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
    context: *mut c_void,
) {
    reply(context, error, |_| {});
}

extern "C" fn browsed(
    _: ServiceRef,
    flags: u32,
    interface: u32,
    error: i32,
    name: *const c_char,
    service: *const c_char,
    domain: *const c_char,
    context: *mut c_void,
) {
    reply(context, error, |replies| {
        if flags & 0x2 == 0 || name.is_null() || service.is_null() || domain.is_null() {
            return;
        }
        // SAFETY: Bonjour supplies NUL-terminated strings valid during the callback.
        let record = unsafe {
            Record {
                interface,
                name: CStr::from_ptr(name).to_owned(),
                service: CStr::from_ptr(service).to_owned(),
                domain: CStr::from_ptr(domain).to_owned(),
            }
        };
        replies.records.push(record);
    });
}

extern "C" fn resolved(
    _: ServiceRef,
    _: u32,
    _: u32,
    error: i32,
    _: *const c_char,
    host: *const c_char,
    port: u16,
    txt_len: u16,
    txt: *const u8,
    context: *mut c_void,
) {
    reply(context, error, |replies| {
        if host.is_null() || txt.is_null() || txt_len == 0 {
            return;
        }
        // SAFETY: Bonjour supplies a string and a TXT buffer of txt_len bytes,
        // both valid for this callback only.
        let (host, txt) = unsafe {
            (
                CStr::from_ptr(host),
                std::slice::from_raw_parts(txt, usize::from(txt_len)),
            )
        };
        if let (Ok(host), Some(fingerprint)) = (host.to_str(), txt_fingerprint(txt)) {
            replies.resolved = Some(Resolved {
                host: host.to_owned(),
                port: u16::from_be(port),
                fingerprint,
            });
        }
    });
}

impl Discovery for MacDiscovery {
    fn announce(&mut self, port: u16, fingerprint: &str) -> Result<(), PlatformError> {
        self.stop_announce();
        let name = CString::new(discovery_instance_name()).map_err(|_| dns_error(-65540))?;
        let txt = txt_record(fingerprint)?;
        let service_type = CString::new(DISCOVERY_SERVICE).map_err(|_| dns_error(-65540))?;
        let replies = Box::<Context>::default();
        let mut service = ptr::null_mut();
        // SAFETY: inputs and boxed context live through registration; Bonjour
        // copies record data. Subsequent replies are processed by the worker.
        let error = unsafe {
            DNSServiceRegister(
                &mut service,
                0,
                0,
                name.as_ptr(),
                service_type.as_ptr(),
                ptr::null(),
                ptr::null(),
                port.to_be(),
                txt.len() as u16,
                txt.as_ptr().cast(),
                registered,
                context_pointer(&replies),
            )
        };
        if error != 0 {
            return Err(dns_error(error));
        }
        self.announce = Some(start_worker(Service(service), replies, self.sink.clone())?);
        Ok(())
    }

    fn stop_announce(&mut self) {
        if let Some(worker) = self.announce.take() {
            worker.stop();
        }
    }

    fn search(&mut self) -> Result<(), PlatformError> {
        self.stop_search();
        let service_type = CString::new(DISCOVERY_SERVICE).map_err(|_| dns_error(-65540))?;
        let replies = Box::<Context>::default();
        let mut service = ptr::null_mut();
        // SAFETY: inputs are valid and the boxed context lives through teardown.
        let error = unsafe {
            DNSServiceBrowse(
                &mut service,
                0,
                0,
                service_type.as_ptr(),
                ptr::null(),
                browsed,
                context_pointer(&replies),
            )
        };
        if error != 0 {
            return Err(dns_error(error));
        }
        self.search = Some(start_worker(Service(service), replies, self.sink.clone())?);
        Ok(())
    }

    fn stop_search(&mut self) {
        if let Some(worker) = self.search.take() {
            worker.stop();
        }
    }
}

impl Drop for MacDiscovery {
    fn drop(&mut self) {
        self.stop_search();
        self.stop_announce();
    }
}

fn start_worker(
    service: Service,
    replies: Box<Context>,
    sink: DiscoverySink,
) -> Result<Worker, PlatformError> {
    let (wake, read) = UnixStream::pair().map_err(io_error)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let thread = thread::Builder::new()
        .name("telescreen-bonjour".into())
        .spawn(move || {
            let result = process(&service, &read, &replies, &stopping, &sink);
            if let Err(error) = result {
                sink(DiscoveryEvent::Failed(error.to_string()));
            }
            // Tear down before dropping the callback context.
            drop(service);
            drop(replies);
        })
        .map_err(io_error)?;
    Ok(Worker { wake, stop, thread })
}

/// Waits for a reply or a stop signal. Resolve uses the same wake socket, so
/// stopping the search never waits for its three-second resolve deadline.
fn process_one(
    service: &Service,
    wake: &UnixStream,
    timeout: c_int,
) -> Result<bool, PlatformError> {
    // SAFETY: service is owned by this thread and remains live during poll.
    let fd = unsafe { DNSServiceRefSockFD(service.0) };
    if fd < 0 {
        return Err(dns_error(-65541));
    }
    let mut fds = [
        PollFd {
            fd,
            events: 1,
            revents: 0,
        },
        PollFd {
            fd: wake.as_raw_fd(),
            events: 1,
            revents: 0,
        },
    ];
    // SAFETY: fds holds two initialized pollfd structures for the whole call.
    let result = unsafe { poll(fds.as_mut_ptr(), 2, timeout) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(true);
        }
        return Err(io_error(error));
    }
    if result == 0 || fds[1].revents != 0 {
        return Ok(false);
    }
    if fds[0].revents & 0x38 != 0 {
        return Err(dns_error(-65563));
    }
    if fds[0].revents & 1 != 0 {
        // SAFETY: only this thread processes the service, with its context alive.
        let error = unsafe { DNSServiceProcessResult(service.0) };
        if error != 0 {
            return Err(dns_error(error));
        }
    }
    Ok(true)
}

fn process(
    service: &Service,
    wake: &UnixStream,
    replies: &Context,
    stop: &Arc<AtomicBool>,
    sink: &DiscoverySink,
) -> Result<(), PlatformError> {
    while !stop.load(Ordering::SeqCst) && process_one(service, wake, -1)? {
        let records = {
            let mut replies = replies.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(error) = replies.error.take() {
                return Err(dns_error(error));
            }
            std::mem::take(&mut replies.records)
        };
        for record in records {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if let Some(found) = resolve_record(record, wake) {
                // Host lookup can block in the system resolver. It owns no
                // service reference and must not delay stop_search on the UI thread.
                let (stop, sink) = (stop.clone(), sink.clone());
                thread::Builder::new()
                    .name("telescreen-bonjour-address".into())
                    .spawn(move || {
                        let Ok(addresses) = (found.host.as_str(), found.port).to_socket_addrs()
                        else {
                            return;
                        };
                        let addresses: Vec<_> =
                            addresses.map(|a| a.ip()).filter(|a| a.is_ipv4()).collect();
                        if !addresses.is_empty() && !stop.load(Ordering::SeqCst) {
                            sink(DiscoveryEvent::Found(FoundPeer {
                                fingerprint: found.fingerprint,
                                addresses,
                                port: found.port,
                            }));
                        }
                    })
                    .map_err(io_error)?;
            }
        }
    }
    Ok(())
}

fn resolve_record(record: Record, wake: &UnixStream) -> Option<Resolved> {
    let replies = Box::<Context>::default();
    let mut service = ptr::null_mut();
    // SAFETY: record strings and context are valid; the context outlives service.
    let error = unsafe {
        DNSServiceResolve(
            &mut service,
            0,
            record.interface,
            record.name.as_ptr(),
            record.service.as_ptr(),
            record.domain.as_ptr(),
            resolved,
            context_pointer(&replies),
        )
    };
    if error != 0 {
        return None;
    }
    let service = Service(service);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        {
            let replies = replies.lock().unwrap_or_else(PoisonError::into_inner);
            if replies.resolved.is_some() || replies.error.is_some() {
                break;
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let timeout = remaining.as_millis().min(3000) as c_int;
        if !process_one(&service, wake, timeout).ok()? {
            break;
        }
    }
    drop(service);
    replies
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .resolved
}

#[cfg(test)]
mod tests {
    use super::{txt_fingerprint, txt_record};

    #[test]
    fn record_has_one_length_prefixed_fingerprint_entry() {
        assert_eq!(txt_record("ab:cd").ok(), Some(b"\x08fp=ab:cd".to_vec()));
        assert!(txt_record(&"x".repeat(253)).is_err());
    }

    #[test]
    fn parser_skips_other_entries_and_rejects_truncated_or_invalid_text() {
        assert_eq!(txt_fingerprint(b"\x03a=b\x05fp=ok"), Some("ok".into()));
        assert_eq!(txt_fingerprint(b"\x05fp=x"), None);
        assert_eq!(txt_fingerprint(b"\x04fp=\xff"), None);
        assert_eq!(txt_fingerprint(b""), None);
    }
}
