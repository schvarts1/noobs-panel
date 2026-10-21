//! The three kinds of stress the benchmarker can apply.
//!
//! Every function reports what happened, so the limits of an instance can be
//! checked from the outside.

use std::ffi::CString;
use std::sync::Mutex;

/// The message the process prints when the kernel stops it for using too
/// much processor time.
///
/// It is written from a signal handler, so it may only use calls that are
/// safe there.
const CPU_LIMIT_MESSAGE: &[u8] =
    b"\n  the cpu limit was reached, the kernel stopped this process\n";

/// The memory that was taken and is held, so the usage stays visible.
///
/// The blocks are kept until the process ends, because the point of the
/// test is that the usage of the instance can be seen while it is high.
static HELD: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Take as much memory as the limits allow.
///
/// The memory is taken with `malloc` instead of the Rust allocator, because
/// the Rust allocator ends the process when it cannot get memory, and the
/// point here is to find out where the limit stops the process, not to end
/// it.
///
/// The memory is held, so the usage of the instance stays high and can be
/// seen on the dashboard.
pub fn memory() -> String {
    const BLOCK: usize = 4 * 1024 * 1024;

    let mut blocks: Vec<usize> = Vec::new();
    let mut total_mb = 0u64;

    loop {
        // SAFETY: the pointer is only used when it is not null, and it is
        // kept in the list of held blocks until the process ends.
        let block = unsafe { libc::malloc(BLOCK) } as *mut u8;
        if block.is_null() {
            break;
        }
        // The block is written to, so the pages are really taken.
        unsafe { std::ptr::write_bytes(block, 1, BLOCK) };
        blocks.push(block as usize);
        total_mb += (BLOCK / (1024 * 1024)) as u64;
    }

    // The memory is kept, so the usage of the instance stays high.
    let held = {
        let mut held = HELD.lock().unwrap();
        held.extend(blocks);
        held.len() as u64 * (BLOCK / (1024 * 1024)) as u64
    };

    format!(
        "The memory limit stopped this process after {total_mb} MB. \
         The instance now holds {held} MB, and the memory is kept until the \
         instance is stopped."
    )
}

/// Use as much processor time as the limits allow.
///
/// The kernel ends the process when the limit is reached, so this does not
/// return in that case. A message is printed first, so the log of the
/// instance shows what happened.
pub fn cpu() -> String {
    // SAFETY: the handler only writes a fixed message, which is safe to do
    // from a signal handler.
    unsafe {
        libc::signal(
            libc::SIGXCPU,
            cpu_limit_handler as *const () as libc::sighandler_t,
        );
    }

    // The work is done in a thread of its own, so the server keeps
    // answering while the processor is busy. The panel can then show what
    // the instance is using.
    std::thread::spawn(|| {
        let mut value: u64 = 0;
        loop {
            // The work is done on a value that is never used, so the
            // compiler cannot throw it away.
            value = value.wrapping_mul(6364136223846793005).wrapping_add(1);
            if value == 0 {
                println!("impossible");
            }
        }
    });

    "The processor is now busy. The instance uses as much processor time as \
     it can, until the limit of the panel stops it."
        .to_string()
}

/// The handler for the cpu limit.
///
/// # Safety
///
/// This is called by the kernel from a signal handler.
unsafe extern "C" fn cpu_limit_handler(_signal: libc::c_int) {
    libc::write(
        1,
        CPU_LIMIT_MESSAGE.as_ptr() as *const libc::c_void,
        CPU_LIMIT_MESSAGE.len(),
    );
    libc::_exit(42);
}

/// Write a file until the size limit stops it.
///
/// The signal for a file that grew too large is ignored, so the write
/// reports the error instead of ending the process.
pub fn disk(directory: &std::path::Path) -> String {
    // SAFETY: ignoring a signal is always safe.
    unsafe {
        libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
    }

    let path = directory.join("disk-test.bin");
    let name = CString::new(path.to_string_lossy().as_bytes()).unwrap_or_default();
    const BLOCK: usize = 1024 * 1024;
    let buffer = vec![0u8; BLOCK];

    // SAFETY: the name is a valid null terminated string.
    let file = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_TRUNC,
            0o644,
        )
    };
    if file < 0 {
        return format!(
            "The file could not be opened: {}",
            std::io::Error::last_os_error()
        );
    }

    let mut written = 0u64;
    let result = loop {
        // SAFETY: the buffer is valid for BLOCK bytes and the file is open.
        let count = unsafe { libc::write(file, buffer.as_ptr() as *const libc::c_void, BLOCK) };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            break Err(error);
        }
        if count == 0 {
            break Ok(());
        }
        written += count as u64;
    };

    // SAFETY: the file is open.
    unsafe {
        libc::close(file);
    }

    match result {
        Ok(()) => format!("The file was written, {written} bytes, and no limit stopped it."),
        Err(error) => format!(
            "The disk limit stopped this process after {} MB. The error was: {error}",
            written / (1024 * 1024)
        ),
    }
}
