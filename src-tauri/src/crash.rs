//! Where Nuzky ended when the end was no panic: a crash in a graphics driver, FFmpeg, whisper.cpp, WebView2 or the
//! system. The signal or exception, the library it happened in, the thread and how long Nuzky had run go into the file
//! that marks this Nuzky as running (diagnostics.rs), so the next start can say where the last one ended. A handler
//! runs while the app falls apart, so it neither allocates nor locks; it writes once and lets the crash go on.
//!
//! What it writes names no file or folder of the person: a library's file name, a thread's name, numbers, and why
//! whisper.cpp stopped, as its code words it.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

struct Target {
    #[cfg(unix)]
    fd: i32,
    #[cfg(windows)]
    handle: usize,
    started: Instant,
    /// Where Nuzky's own executable is loaded. A crash in it names Nuzky, never the file, which a person may rename.
    exe_base: usize,
    #[cfg(windows)]
    main_thread: u32,
}

/// What a crash in Nuzky's own executable names as its library.
const OWN: &str = "Nuzky";

static TARGET: OnceLock<Target> = OnceLock::new();
static CRASHED: AtomicBool = AtomicBool::new(false);
static OUT_OF_MEMORY: AtomicBool = AtomicBool::new(false);

/// Writes how this Nuzky crashes into `marker`, which it holds open until it quits.
pub fn install(marker: &std::fs::File) {
    let target = Target {
        #[cfg(unix)]
        fd: std::os::fd::AsRawFd::as_raw_fd(marker),
        #[cfg(windows)]
        handle: std::os::windows::io::AsRawHandle::as_raw_handle(marker) as usize,
        started: Instant::now(),
        exe_base: platform::exe_base(),
        #[cfg(windows)]
        // SAFETY: takes no arguments.
        main_thread: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
    };
    if TARGET.set(target).is_ok() {
        platform::install();
        nuzky_analysis::on_abort(note);
    }
}

/// Why whisper.cpp stops the app, which it says just before it aborts.
fn note(message: &str) {
    let mut line = Line::new();
    line.push("note: ");
    line.value(message);
    line.write();
}

/// The record, or a line of it, on the stack and cut at its size.
struct Line {
    bytes: [u8; 512],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Self { bytes: [0; 512], len: 0 }
    }

    fn push(&mut self, text: &str) {
        let mut n = text.len().min(self.bytes.len() - 1 - self.len);
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        self.bytes[self.len..self.len + n].copy_from_slice(&text.as_bytes()[..n]);
        self.len += n;
    }

    fn text(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }

    /// Text from elsewhere, kept on its line.
    fn value(&mut self, text: &str) {
        for c in text.chars() {
            let mut utf8 = [0; 4];
            self.push(if c.is_control() { " " } else { c.encode_utf8(&mut utf8) });
        }
    }

    fn write(mut self) {
        self.push("\n");
        if let Some(target) = TARGET.get() {
            platform::write(target, &self.bytes[..self.len]);
        }
    }
}

impl std::fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.push(text);
        Ok(())
    }
}

/// The crash and how long Nuzky had run, then the library it happened in and the thread. Finding the library can
/// wait on a lock of the system's loader, which a crashed thread may hold, so it comes after what is surely known,
/// within a limit past which the app ends rather than hangs.
fn record(
    crash: impl FnOnce(&mut Line),
    place: impl FnOnce() -> Option<(Line, usize)>,
    thread: impl FnOnce(&mut Line),
) {
    let mut line = Line::new();
    line.push("crash: ");
    crash(&mut line);
    if let Some(target) = TARGET.get() {
        write!(line, "\nafter: {}", target.started.elapsed().as_secs()).ok();
    }
    line.write();
    platform::within_limit(|| {
        if let Some((name, offset)) = place() {
            let mut line = Line::new();
            line.push("in: ");
            line.push(name.text());
            write!(line, " +0x{offset:x}").ok();
            line.write();
        }
        let mut line = Line::new();
        line.push("thread: ");
        thread(&mut line);
        line.write();
    });
}

/// The record a crashed Nuzky left, as a report and Help improve Nuzky tell it.
#[derive(Debug, Default, PartialEq)]
pub struct Crash {
    pub kind: String,
    pub library: Option<String>,
    pub offset: Option<String>,
    pub thread: Option<String>,
    pub after_seconds: Option<u64>,
    /// Why whisper.cpp stopped the app.
    pub note: Option<String>,
    /// Bytes of an allocation that failed before it ended.
    pub failed_allocation: Option<u64>,
}

impl Crash {
    /// What a marker holds; None when the crash wrote nothing, as when the process was killed or the power went.
    pub fn read(text: &str) -> Option<Self> {
        let mut crash = Crash::default();
        let clean = |value: &str, chars: usize| -> String {
            let value: String = value.chars().filter(|c| !c.is_control()).take(chars).collect();
            crate::diagnostics::scrub_names(value.trim())
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once(": ") else { continue };
            match key {
                "crash" if crash.kind.is_empty() => crash.kind = clean(value, 80),
                "in" if crash.library.is_none() => {
                    let (library, offset) = value.rsplit_once(" +0x").unwrap_or((value, ""));
                    crash.library = Some(clean(library, 80));
                    crash.offset = offset.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("0x{offset}"));
                }
                "thread" if crash.thread.is_none() => crash.thread = Some(clean(value, 80)).filter(|t| !t.is_empty()),
                "after" => crash.after_seconds = value.trim().parse().ok(),
                "note" if crash.note.is_none() => {
                    // ggml names the source file by the folders of the computer that built it.
                    let note = match value.split_once(": ") {
                        Some((place, why)) => format!("{}: {why}", place.rsplit(['/', '\\']).next().unwrap_or(place)),
                        None => value.to_owned(),
                    };
                    crash.note = Some(clean(&note, 400));
                }
                "memory" if crash.failed_allocation.is_none() => crash.failed_allocation = value.trim().parse().ok(),
                _ => {}
            }
        }
        (!crash.kind.is_empty() || crash.failed_allocation.is_some()).then_some(crash)
    }

    /// One line for the report, such as "It crashed in igvk64.dll at +0x64ea72 on the preview-engine thread, 5 s
    /// after starting: access violation writing memory."
    pub fn sentence(&self) -> String {
        if self.kind.is_empty() {
            let bytes = self.failed_allocation.unwrap_or_default();
            return format!("No crash was noted, but before it ended {} of memory could not be had.", size(bytes));
        }
        let mut text = "It crashed".to_owned();
        if let Some(library) = &self.library {
            text += &format!(" in {library}");
            if let Some(offset) = &self.offset {
                text += &format!(" at +{offset}");
            }
        }
        if let Some(thread) = &self.thread {
            text += &format!(" on the {thread} thread");
        }
        if let Some(seconds) = self.after_seconds {
            text += &format!(", {} after starting", duration(seconds));
        }
        text += &format!(": {}", self.kind);
        text.push('.');
        if let Some(note) = &self.note {
            text += &format!(" whisper.cpp said: {note}");
        }
        if let Some(bytes) = self.failed_allocation {
            text += &format!(" Before that, {} of memory could not be had.", size(bytes));
        }
        text
    }

    pub fn facts(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind, "library": self.library, "offset": self.offset, "thread": self.thread,
            "afterSeconds": self.after_seconds, "note": self.note, "failedAllocation": self.failed_allocation,
        })
    }
}

fn duration(seconds: u64) -> String {
    match seconds {
        0..120 => format!("{seconds} s"),
        120..7200 => format!("{} min", seconds / 60),
        _ => format!("{} h", seconds / 3600),
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        0..0x100000 => format!("{} KB", bytes.div_ceil(1024)),
        0x100000..0x40000000 => format!("{} MB", bytes >> 20),
        _ => format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64),
    }
}

/// Rust stops the app on a failed allocation without a panic, and on Windows past every handler, so the allocator
/// notes the first one itself.
pub struct Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn checked(memory: *mut u8, bytes: usize) -> *mut u8 {
    if memory.is_null() && !OUT_OF_MEMORY.swap(true, Ordering::AcqRel) {
        let mut line = Line::new();
        write!(line, "memory: {bytes}").ok();
        line.write();
    }
    memory
}

// SAFETY: every call goes to the system allocator unchanged; only a failure is noted.
unsafe impl std::alloc::GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        checked(unsafe { std::alloc::System.alloc(layout) }, layout.size())
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        checked(unsafe { std::alloc::System.alloc_zeroed(layout) }, layout.size())
    }

    unsafe fn dealloc(&self, memory: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(memory, layout) }
    }

    unsafe fn realloc(&self, memory: *mut u8, layout: std::alloc::Layout, bytes: usize) -> *mut u8 {
        checked(unsafe { std::alloc::System.realloc(memory, layout, bytes) }, bytes)
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{CStr, c_int, c_void};
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use super::{CRASHED, Line, OWN, TARGET, Target, record};

    const SIGNALS: [(c_int, &str); 6] = [
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGTRAP, "SIGTRAP"),
    ];

    /// The handlers before, such as Rust's own that tells a stack overflow.
    static PREVIOUS: OnceLock<[libc::sigaction; SIGNALS.len()]> = OnceLock::new();

    pub fn write(target: &Target, bytes: &[u8]) {
        // SAFETY: write is async-signal-safe and the file stays open while the app runs.
        unsafe { libc::write(target.fd, bytes.as_ptr().cast(), bytes.len()) };
    }

    pub fn exe_base() -> usize {
        // SAFETY: an all-zero Dl_info is valid; dladdr fills it in for a function of this executable.
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        unsafe { libc::dladdr(exe_base as *const c_void, &mut info) };
        info.dli_fbase as usize
    }

    /// Runs `f`, which a SIGALRM after two seconds ends together with the app.
    pub fn within_limit(f: impl FnOnce()) {
        // SAFETY: signal and alarm are async-signal-safe; SIGALRM's default action ends the app.
        unsafe {
            libc::signal(libc::SIGALRM, libc::SIG_DFL);
            libc::alarm(2);
        }
        f();
        // SAFETY: as above.
        unsafe { libc::alarm(0) };
    }

    pub fn install() {
        // SAFETY: an all-zero sigaction is valid; sigaction fills in each previous one.
        let mut previous: [libc::sigaction; SIGNALS.len()] = unsafe { std::mem::zeroed() };
        for (n, (signal, _)) in SIGNALS.iter().enumerate() {
            // SAFETY: as above.
            unsafe { libc::sigaction(*signal, std::ptr::null(), &mut previous[n]) };
        }
        if PREVIOUS.set(previous).is_err() {
            return;
        }
        for (signal, _) in SIGNALS {
            // SAFETY: `on_signal` only makes calls a signal handler may make; the alternate stack Rust gives its
            // threads lets it run after a stack overflow.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        }
    }

    extern "C" fn on_signal(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
        let index = SIGNALS.iter().position(|(s, _)| *s == signal).unwrap_or(0);
        // Another process's `kill -SEGV` is no crash here, and Rust's handler lets the app go on after it. An abort
        // counts however it came, as abort() raises it as if sent.
        // SAFETY: the kernel passes the signal's information.
        let sent = signal != libc::SIGABRT && !info.is_null() && sent_by_a_process(unsafe { (*info).si_code });
        if !sent && !CRASHED.swap(true, Ordering::AcqRel) {
            // SAFETY: the kernel passes the interrupted thread's context.
            unsafe { note_signal(signal, SIGNALS[index].1, context) };
        }
        let previous = PREVIOUS.get().map(|all| all[index]);
        // SAFETY: calls the handler that was there before with what this one got, or ends the app as the signal
        // would have: raised now with the default action, it ends the app once this handler returns.
        unsafe {
            match previous {
                Some(p) if p.sa_sigaction != libc::SIG_DFL && p.sa_sigaction != libc::SIG_IGN => {
                    if p.sa_flags & libc::SA_SIGINFO != 0 {
                        let handler: extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) =
                            std::mem::transmute(p.sa_sigaction);
                        handler(signal, info, context);
                    } else {
                        let handler: extern "C" fn(c_int) = std::mem::transmute(p.sa_sigaction);
                        handler(signal);
                    }
                }
                _ => {
                    libc::signal(signal, libc::SIG_DFL);
                    libc::raise(signal);
                }
            }
        }
    }

    unsafe fn note_signal(signal: c_int, name: &str, context: *mut c_void) {
        let kind = |line: &mut Line| line.push(name);
        // An abort's instruction is in the C library; who called it is not known here.
        let place = || (signal != libc::SIGABRT).then(|| unsafe { library_of(instruction(context)) }).flatten();
        record(kind, place, |line| unsafe { thread_name(line) });
    }

    fn sent_by_a_process(code: c_int) -> bool {
        #[cfg(target_os = "linux")]
        return code <= 0;
        // macOS gives kill() 0 rather than SI_USER and SI_QUEUE, which its <sys/signal.h> names and the libc crate
        // leaves out.
        #[cfg(not(target_os = "linux"))]
        return code <= 0 || code == 0x10001 || code == 0x10002;
    }

    /// The instruction the thread was at.
    unsafe fn instruction(context: *mut c_void) -> usize {
        if context.is_null() {
            return 0;
        }
        let context = context as *const libc::ucontext_t;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return unsafe { (*context).uc_mcontext.gregs[libc::REG_RIP as usize] as usize };
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        return unsafe { (*context).uc_mcontext.pc as usize };
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return unsafe { (*(*context).uc_mcontext).__ss.__pc as usize };
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        return unsafe { (*(*context).uc_mcontext).__ss.__rip as usize };
        #[allow(unreachable_code)]
        0
    }

    /// The file name of the library `address` is in, and how far into it.
    unsafe fn library_of(address: usize) -> Option<(Line, usize)> {
        if address == 0 {
            return None;
        }
        // SAFETY: an all-zero Dl_info is valid; dladdr fills it in.
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        if unsafe { libc::dladdr(address as *const c_void, &mut info) } == 0 || info.dli_fname.is_null() {
            return None;
        }
        let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_bytes();
        let file = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
        let mut name = Line::new();
        if TARGET.get().is_some_and(|target| target.exe_base == info.dli_fbase as usize) {
            name.push(OWN);
        } else {
            name.value(std::str::from_utf8(file).unwrap_or("a library"));
        }
        Some((name, address.wrapping_sub(info.dli_fbase as usize)))
    }

    unsafe fn thread_name(line: &mut Line) {
        let mut name = [0 as libc::c_char; 64];
        // SAFETY: the name of the calling thread into a buffer of the given size.
        let named = unsafe { libc::pthread_getname_np(libc::pthread_self(), name.as_mut_ptr(), name.len()) } == 0;
        let name = if named { unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes() } else { b"" };
        if unsafe { is_main_thread() } {
            line.push("main");
        } else if let Some(name) = std::str::from_utf8(name).ok().filter(|name| !name.is_empty()) {
            line.value(name);
        } else {
            line.push("unnamed");
        }
    }

    unsafe fn is_main_thread() -> bool {
        #[cfg(target_os = "linux")]
        return unsafe { libc::syscall(libc::SYS_gettid) == libc::getpid() as libc::c_long };
        #[cfg(target_os = "macos")]
        return unsafe { libc::pthread_main_np() } == 1;
        #[allow(unreachable_code)]
        false
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;

    use windows_sys::Wdk::System::Threading::{NtQueryInformationThread, ThreadNameInformation};
    use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, UNICODE_STRING};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, RtlPcToFileHeader, SetUnhandledExceptionFilter,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId};

    use super::{CRASHED, Line, OWN, TARGET, Target, record};

    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
    const SIGABRT: i32 = 22;

    unsafe extern "C" {
        /// The C runtime's, which C and C++ libraries reach through abort().
        fn signal(signal: i32, handler: usize) -> usize;
    }

    static PREVIOUS: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();

    pub fn write(target: &Target, bytes: &[u8]) {
        let mut written = 0;
        // SAFETY: the marker's handle stays open while the app runs.
        unsafe {
            WriteFile(target.handle as _, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut())
        };
    }

    pub fn exe_base() -> usize {
        // SAFETY: the module handle of this executable, which stays loaded.
        unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) as usize }
    }

    /// Runs `f`. Windows looks the library up the way it did to dispatch the exception, and the thread's name is read
    /// into the stack, so nothing here waits on the heap.
    pub fn within_limit(f: impl FnOnce()) {
        f();
    }

    pub fn install() {
        // SAFETY: both handlers only write the record and pass the crash on.
        unsafe {
            PREVIOUS.set(SetUnhandledExceptionFilter(Some(on_exception))).ok();
            signal(SIGABRT, on_abort as *const () as usize);
        }
    }

    unsafe extern "system" fn on_exception(pointers: *const EXCEPTION_POINTERS) -> i32 {
        if !CRASHED.swap(true, Ordering::AcqRel) && !pointers.is_null() {
            // SAFETY: Windows passes the exception's record.
            unsafe { note_exception(&*(*pointers).ExceptionRecord) };
        }
        match PREVIOUS.get().copied().flatten() {
            // SAFETY: the filter that was there before, with what this one got.
            Some(previous) => unsafe { previous(pointers) },
            None => EXCEPTION_CONTINUE_SEARCH,
        }
    }

    extern "C" fn on_abort(_: i32) {
        if !CRASHED.swap(true, Ordering::AcqRel) {
            record(|line| line.push("abort"), || None, |line| unsafe { thread_name(line) });
        }
    }

    unsafe fn note_exception(exception: &windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_RECORD) {
        let code = exception.ExceptionCode;
        let kind = |line: &mut Line| match code {
            EXCEPTION_ACCESS_VIOLATION => line.push(match exception.ExceptionInformation[0] {
                0 => "access violation reading memory",
                1 => "access violation writing memory",
                8 => "access violation running memory that holds no code",
                _ => "access violation",
            }),
            _ => match code as u32 {
                0xC00000FD => line.push("stack overflow"),
                0xC000001D => line.push("illegal instruction"),
                0xC0000094 => line.push("integer division by zero"),
                0xC0000006 => line.push("a file it was reading became unreadable"),
                0xC0000374 => line.push("heap corruption"),
                0xC06D007E => line.push("a library it needed did not load"),
                0xC06D007F => line.push("a function it needed is missing from a library"),
                0xE06D7363 => line.push("an unhandled C++ exception"),
                0x80000003 => line.push("a breakpoint"),
                other => {
                    use std::fmt::Write as _;
                    write!(line, "exception 0x{other:08X}").ok();
                }
            },
        };
        let address = exception.ExceptionAddress as usize;
        record(kind, || unsafe { library_of(address) }, |line| unsafe { thread_name(line) });
    }

    /// The file name of the module `address` is in, from its export table, and how far into it. The lookup is the one
    /// Windows made to dispatch the exception, and reading the name takes no lock.
    unsafe fn library_of(address: usize) -> Option<(Line, usize)> {
        let target = TARGET.get()?;
        let mut base: *mut c_void = std::ptr::null_mut();
        // SAFETY: looks the address up among the loaded modules.
        if unsafe { RtlPcToFileHeader(address as *const c_void, &mut base) }.is_null() || base.is_null() {
            return None;
        }
        let base = base as usize;
        let mut name = Line::new();
        if base == target.exe_base {
            name.push(OWN);
        } else if let Some(exported) =
            unsafe { export_name(base as *const u8) }.and_then(|n| std::str::from_utf8(n).ok())
        {
            name.value(exported);
        } else {
            name.push("a library without a name");
        }
        Some((name, address - base))
    }

    /// The name a loaded DLL gives itself in its export table.
    unsafe fn export_name(base: *const u8) -> Option<&'static [u8]> {
        let read_u16 = |at: *const u8| unsafe { at.cast::<u16>().read_unaligned() };
        let read_u32 = |at: *const u8| unsafe { at.cast::<u32>().read_unaligned() };
        let headers = unsafe { base.add(read_u32(base.add(0x3c)) as usize) };
        if read_u32(headers) != 0x4550 {
            return None;
        }
        let optional = unsafe { headers.add(24) };
        let (count, directories) = match read_u16(optional) {
            0x20b => (unsafe { optional.add(108) }, unsafe { optional.add(112) }),
            0x10b => (unsafe { optional.add(92) }, unsafe { optional.add(96) }),
            _ => return None,
        };
        let exports = read_u32(directories) as usize;
        if read_u32(count) == 0 || exports == 0 {
            return None;
        }
        let name = unsafe { base.add(read_u32(base.add(exports + 12)) as usize) };
        let len = (0..260).take_while(|&i| unsafe { *name.add(i) } != 0).count();
        (len > 0).then(|| unsafe { std::slice::from_raw_parts(name, len) })
    }

    /// The thread's description, which Rust sets for the threads it starts. Windows writes it into `buffer` as a
    /// UNICODE_STRING followed by its text; GetThreadDescription would copy it into memory from the heap.
    unsafe fn thread_name(line: &mut Line) {
        let mut buffer = [0u64; 48];
        let mut length = 0;
        // SAFETY: the buffer is aligned for a UNICODE_STRING and its size is given.
        let read = unsafe {
            NtQueryInformationThread(
                GetCurrentThread(),
                ThreadNameInformation,
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as u32,
                &mut length,
            )
        } >= 0;
        // SAFETY: on success the buffer starts with the UNICODE_STRING, whose text lies in the rest of it.
        let name = read.then(|| unsafe { *buffer.as_ptr().cast::<UNICODE_STRING>() });
        let units = name
            .filter(|name| !name.Buffer.is_null() && name.Length > 0)
            .map(|name| unsafe { std::slice::from_raw_parts(name.Buffer, usize::from(name.Length / 2)) });
        if let Some(units) = units {
            for c in char::decode_utf16(units.iter().copied()) {
                let mut utf8 = [0; 4];
                line.value(c.unwrap_or('?').encode_utf8(&mut utf8));
            }
        } else if TARGET.get().is_some_and(|t| t.main_thread == unsafe { GetCurrentThreadId() }) {
            line.push("main");
        } else {
            line.push("unnamed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_as_where_and_why_it_ended() {
        let windows =
            "crash: access violation writing memory\nin: igvk64.dll +0x64ea72\nthread: preview-engine\nafter: 5\n";
        let crash = Crash::read(windows).unwrap();
        assert_eq!(
            crash.sentence(),
            "It crashed in igvk64.dll at +0x64ea72 on the preview-engine thread, 5 s after starting: access violation \
             writing memory."
        );
        let speech = "note: D:\\a\\nuzky\\whisper.cpp\\ggml\\src\\ggml-vulkan.cpp:5120: GGML_ASSERT(buf) failed\n\
                      crash: abort\nthread: speech\nafter: 4000\n";
        assert_eq!(
            Crash::read(speech).unwrap().sentence(),
            "It crashed on the speech thread, 66 min after starting: abort. whisper.cpp said: ggml-vulkan.cpp:5120: \
             GGML_ASSERT(buf) failed"
        );
        let out_of_memory = "memory: 3221225472\ncrash: SIGABRT\nafter: 9000\nthread: main\n";
        assert_eq!(
            Crash::read(out_of_memory).unwrap().sentence(),
            "It crashed on the main thread, 2 h after starting: SIGABRT. Before that, 3.0 GB of memory could not be had."
        );
        assert_eq!(
            Crash::read("memory: 52428800\n").unwrap().sentence(),
            "No crash was noted, but before it ended 50 MB of memory could not be had."
        );
        assert_eq!(Crash::read(""), None, "killed or the power went: nothing to say");
    }

    const CHILD: &str = "NUZKY_CRASH_CHILD";

    /// Run by `a_native_crash_names_its_library_and_thread` in a process of its own, which it crashes. The handler
    /// there before ends it with code 77, so the crash leaves no report with the system.
    #[test]
    #[ignore = "crashes its process; the test below runs it"]
    fn crashing_child() {
        let Some(marker) = std::env::var_os(CHILD) else { return };
        let file = std::fs::File::create(marker).unwrap();
        #[cfg(unix)]
        // SAFETY: _exit may be called from a signal handler.
        unsafe {
            extern "C" fn quit(_: libc::c_int) {
                unsafe { libc::_exit(77) }
            }
            libc::signal(libc::SIGSEGV, quit as *const () as libc::sighandler_t);
        }
        #[cfg(windows)]
        // SAFETY: ends the process from the exception filter.
        unsafe {
            unsafe extern "system" fn quit(
                _: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
            ) -> i32 {
                unsafe { windows_sys::Win32::System::Threading::ExitProcess(77) }
            }
            windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(Some(quit));
        }
        install(&file);
        unsafe extern "C" {
            fn strlen(text: *const std::ffi::c_char) -> usize;
        }
        std::thread::Builder::new()
            .name("preview-engine".into())
            .spawn(|| {
                // SAFETY: none; the C library reads through a null pointer, as a driver does when it crashes.
                unsafe { strlen(std::hint::black_box(std::ptr::null())) };
            })
            .unwrap()
            .join()
            .ok();
    }

    #[test]
    fn a_native_crash_names_its_library_and_thread() {
        let dir = std::env::temp_dir().join(format!("nuzky-native-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("running-1");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash::tests::crashing_child", "--ignored", "--nocapture"])
            .env(CHILD, &marker)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77), "the child crashed and the handler before was called");
        let text = std::fs::read_to_string(&marker).unwrap();
        let crash = Crash::read(&text).unwrap_or_else(|| panic!("nothing recorded: {text:?}"));
        let exe = std::env::current_exe().unwrap();
        let exe = exe.file_name().unwrap().to_string_lossy();
        assert_eq!(crash.kind, if cfg!(windows) { "access violation reading memory" } else { "SIGSEGV" }, "{text}");
        let library = crash.library.as_deref().unwrap_or_default();
        assert!(
            !library.is_empty() && library != exe && !library.contains(['/', '\\']),
            "the C library by name: {text}"
        );
        assert!(crash.offset.is_some(), "{text}");
        assert_eq!(crash.thread.as_deref(), Some("preview-engine"), "{text}");
        assert_eq!(crash.after_seconds, Some(0), "{text}");
        std::fs::remove_dir_all(dir).ok();
    }
}
