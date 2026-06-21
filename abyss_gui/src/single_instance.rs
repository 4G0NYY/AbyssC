//! One window, many files.
//!
//! Windows fires a context-menu verb once per selected file, so "Compress with
//! AbyssC" on five files would otherwise spawn five windows. To make a
//! multi-select mean "one archive", the first process to launch claims a
//! loopback socket and becomes the *primary*; later launches connect to it,
//! hand over their path, and exit. The primary collects them all.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Loopback rendezvous address. Loopback binds are firewall-exempt, so this
/// never prompts the user.
const ADDR: &str = "127.0.0.1:51797";

/// A shared queue of paths forwarded by secondary instances.
pub type Inbox = Arc<Mutex<VecDeque<PathBuf>>>;

/// The role this process took.
pub enum Instance {
    /// We own the socket — run the window and accept forwarded paths.
    Primary(TcpListener),
    /// Another instance accepted our paths; the caller should exit immediately.
    Forwarded,
    /// We could neither bind nor forward — run a normal standalone window.
    Standalone,
}

/// Claim the primary role, or forward `paths` to an existing primary.
pub fn acquire(paths: &[PathBuf]) -> Instance {
    match TcpListener::bind(ADDR) {
        Ok(listener) => Instance::Primary(listener),
        Err(_) => {
            if paths.is_empty() {
                // No payload to hand off (e.g. an extract/browse launch): just
                // open our own window alongside whatever is already running.
                return Instance::Standalone;
            }
            // The primary may still be starting its listener; retry briefly.
            for _ in 0..15 {
                if let Ok(mut stream) = TcpStream::connect(ADDR) {
                    let mut payload = String::new();
                    for p in paths {
                        payload.push_str(&p.to_string_lossy());
                        payload.push('\n');
                    }
                    let _ = stream.write_all(payload.as_bytes());
                    let _ = stream.flush();
                    return Instance::Forwarded;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Instance::Standalone
        }
    }
}

/// Spawn the accept loop: every path a secondary forwards is pushed into `inbox`.
pub fn serve(listener: TcpListener, inbox: Inbox) {
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let mut buf = String::new();
            // Bound the read so a misbehaving peer can't exhaust memory.
            let _ = stream.take(64 * 1024).read_to_string(&mut buf);
            if let Ok(mut queue) = inbox.lock() {
                for line in buf.lines() {
                    let line = line.trim();
                    if !line.is_empty() {
                        queue.push_back(PathBuf::from(line));
                    }
                }
            }
        }
    });
}
