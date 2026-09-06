use std::{
    io::{self, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};

#[cfg(unix)]
pub(super) trait PipeIo: AsRawFd {}
#[cfg(unix)]
impl<T: AsRawFd> PipeIo for T {}

#[cfg(windows)]
pub(super) trait PipeIo {}
#[cfg(windows)]
impl<T> PipeIo for T {}

#[cfg(unix)]
pub(super) fn set_nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: the borrowed pipe owns a valid descriptor for both fcntl calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn wait_ready(fd: RawFd, events: libc::c_short, stop: &AtomicBool) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    while !stop.load(Ordering::SeqCst) {
        // SAFETY: descriptor is initialized and valid for this one-element poll.
        let result = unsafe { libc::poll(&mut descriptor, 1, 100) };
        if result > 0 {
            return Ok(!stop.load(Ordering::SeqCst));
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    Ok(false)
}

// Descendants can retain either pipe after the shell dies. Every read and write
// must therefore be cancellable, including writes blocked by pipe backpressure.
pub(super) fn copy<R: Read + PipeIo, W: Write + PipeIo>(
    reader: &mut R,
    writer: &mut W,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut buffer = [0; 8192];
    while !stop.load(Ordering::SeqCst) {
        #[cfg(unix)]
        if !wait_ready(reader.as_raw_fd(), libc::POLLIN, stop)? {
            return Ok(());
        }
        let count = match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => count,
            Err(error) if retryable(&error) => continue,
            Err(error) => return Err(error),
        };
        let mut written = 0;
        while written < count && !stop.load(Ordering::SeqCst) {
            #[cfg(unix)]
            if !wait_ready(writer.as_raw_fd(), libc::POLLOUT, stop)? {
                return Ok(());
            }
            match writer.write(&buffer[written..count]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(size) => written += size,
                Err(error) if retryable(&error) => continue,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

pub(super) fn join_workers(workers: &mut Vec<thread::JoinHandle<()>>) {
    #[cfg(windows)]
    {
        use std::{os::windows::thread::JoinHandleExt, time::Duration};
        use windows_sys::Win32::System::IO::CancelSynchronousIo;

        // Repeat cancellation to cover a worker entering ReadFile/WriteFile
        // just after checking stop. These threads only perform our proxy I/O.
        while workers.iter().any(|worker| !worker.is_finished()) {
            for worker in workers.iter().filter(|worker| !worker.is_finished()) {
                // SAFETY: the owned JoinHandle keeps this thread handle valid.
                // ERROR_NOT_FOUND is expected if it has no pending I/O yet.
                unsafe { CancelSynchronousIo(worker.as_raw_handle()) };
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    for worker in workers.drain(..) {
        let _ = worker.join();
    }
}
