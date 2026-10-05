//! One running BergPDF: a second start hands its files to the first and exits.
//!
//! The first instance listens on a loopback port and records the port and a random token in a
//! private file in the data folder. A later start reads that file, connects, sends the token and the
//! absolute paths to open, and gets an acknowledgement; only then does it quit. A stale file (the
//! first instance crashed), a closed port, a wrong reply or a hung peer all end in "become the first
//! instance instead", so a leftover can never stop BergPDF from starting.
//!
//! The token keeps other programs (and other users of the same computer) from making BergPDF open
//! files, since a loopback port is reachable by every local account.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const MAGIC: &str = "BERGPDF1";
const ACK: &str = "BERGPDF-OK";
const LOCK_FILE: &str = "instance.lock";
/// Most paths accepted in one message.
const MAX_PATHS: usize = 64;
/// Most bytes read from a connection.
const MAX_BYTES: u64 = 1 << 20;

/// The first instance: files other starts ask it to open arrive on `rx` (an empty list means
/// "just come to the front").
pub struct Server {
    /// Messages from later starts.
    pub rx: Receiver<Vec<PathBuf>>,
}

/// What [`start`] decided.
pub enum Start {
    /// This process is the first instance.
    Primary(Server),
    /// Another instance took over the files; this process should exit.
    Forwarded,
}

/// Become the first instance or hand `files` (absolute paths) to the running one. `wake` is called
/// on the server thread whenever a message arrives (use it to request a repaint).
pub fn start(files: &[PathBuf], wake: Arc<dyn Fn() + Send + Sync>) -> Start {
    start_in(&crate::dirs::data_dir(), files, wake)
}

/// [`start`] with an explicit folder for the lock file (for tests).
pub fn start_in(dir: &Path, files: &[PathBuf], wake: Arc<dyn Fn() + Send + Sync>) -> Start {
    let lock = dir.join(LOCK_FILE);
    if try_forward(&lock, files) {
        return Start::Forwarded;
    }
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        // No loopback networking: run without single-instance behaviour.
        return Start::Primary(Server { rx: channel().1 });
    };
    let port = listener.local_addr().map_or(0, |a| a.port());
    let token = new_token();
    if let Err(e) = write_private(&lock, &format!("{port}\n{token}\n")) {
        tracing::warn!("single-instance lock file not written: {e}");
    }
    let (tx, rx) = channel();
    std::thread::spawn(move || serve(&listener, &token, &tx, &*wake));
    Start::Primary(Server { rx })
}

fn new_token() -> String {
    // `RandomState` is seeded from the operating system's randomness for every instance.
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn try_forward(lock: &Path, files: &[PathBuf]) -> bool {
    let Ok(text) = std::fs::read_to_string(lock) else {
        return false;
    };
    let mut lines = text.lines();
    let (Some(port), Some(token)) = (
        lines.next().and_then(|p| p.trim().parse::<u16>().ok()),
        lines.next().map(str::trim),
    ) else {
        return false;
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    let paths: Vec<&str> = files
        .iter()
        .filter_map(|p| p.to_str())
        .filter(|p| !p.contains(['\n', '\r']))
        .take(MAX_PATHS)
        .collect();
    let mut msg = format!("{MAGIC}\n{token}\n{}\n", paths.len());
    for p in &paths {
        msg.push_str(p);
        msg.push('\n');
    }
    if s.write_all(msg.as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == ACK
}

fn serve(
    listener: &TcpListener,
    token: &str,
    tx: &Sender<Vec<PathBuf>>,
    wake: &(dyn Fn() + Send + Sync),
) {
    for conn in listener.incoming() {
        let Ok(mut s) = conn else { continue };
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let Some(paths) = read_message(&mut s, token) else {
            continue;
        };
        if s.write_all(format!("{ACK}\n").as_bytes()).is_err() {
            continue;
        }
        if tx.send(paths).is_err() {
            return; // the application is gone
        }
        wake();
    }
}

fn read_message(s: &mut TcpStream, token: &str) -> Option<Vec<PathBuf>> {
    let mut r = BufReader::new(s.take(MAX_BYTES));
    let mut line = String::new();
    let mut next = |r: &mut BufReader<_>| -> Option<String> {
        line.clear();
        (r.read_line(&mut line).ok()? > 0).then(|| line.trim_end_matches(['\r', '\n']).to_string())
    };
    if next(&mut r)? != MAGIC || next(&mut r)? != token {
        return None;
    }
    let n: usize = next(&mut r)?.parse().ok()?;
    if n > MAX_PATHS {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let p = PathBuf::from(next(&mut r)?);
        // Only absolute paths: a relative one would mean something different in this process.
        if !p.is_absolute() {
            return None;
        }
        out.push(p);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake() -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }

    #[test]
    fn a_second_start_hands_its_files_to_the_first_and_quits() {
        let dir = tempfile::tempdir().unwrap();
        let Start::Primary(server) = start_in(dir.path(), &[], wake()) else {
            panic!("the first start must be the primary");
        };
        let file = std::env::temp_dir().join("one.pdf");
        let second = start_in(dir.path(), std::slice::from_ref(&file), wake());
        assert!(matches!(second, Start::Forwarded));
        let got = server.rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(got, vec![file]);
        // A start without files just asks the first instance to come to the front.
        assert!(matches!(
            start_in(dir.path(), &[], wake()),
            Start::Forwarded
        ));
        assert!(
            server
                .rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_stale_lock_file_never_blocks_a_start() {
        let dir = tempfile::tempdir().unwrap();
        // Nobody listens on this port (a listener was bound and dropped), and the token is junk.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::write(dir.path().join(LOCK_FILE), format!("{port}\nabc\n")).unwrap();
        assert!(matches!(
            start_in(dir.path(), &[], wake()),
            Start::Primary(_)
        ));
        // Garbage in the lock file is also ignored.
        std::fs::write(dir.path().join(LOCK_FILE), "not a lock").unwrap();
        assert!(matches!(
            start_in(dir.path(), &[], wake()),
            Start::Primary(_)
        ));
    }

    #[test]
    fn a_wrong_token_or_relative_path_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let Start::Primary(server) = start_in(dir.path(), &[], wake()) else {
            panic!()
        };
        let lock = std::fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
        let mut it = lock.lines();
        let port: u16 = it.next().unwrap().parse().unwrap();
        let token = it.next().unwrap().to_string();
        let send = |msg: &str| -> String {
            let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            s.write_all(msg.as_bytes()).unwrap();
            let _ = s.shutdown(std::net::Shutdown::Write);
            let mut reply = String::new();
            let _ = BufReader::new(s).read_line(&mut reply);
            reply
        };
        let abs = std::env::temp_dir().join("ok.pdf");
        let abs_s = abs.to_str().unwrap();
        assert_eq!(send(&format!("{MAGIC}\nwrong\n1\n{abs_s}\n")), "");
        assert_eq!(send(&format!("{MAGIC}\n{token}\n1\nrelative.pdf\n")), "");
        assert_eq!(send("GET / HTTP/1.1\r\n\r\n"), "");
        assert!(server.rx.recv_timeout(Duration::from_millis(300)).is_err());
        assert_eq!(send(&format!("{MAGIC}\n{token}\n1\n{abs_s}\n")).trim(), ACK);
        assert_eq!(
            server.rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            vec![abs]
        );
    }
}
