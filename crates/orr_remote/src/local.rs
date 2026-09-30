//! [`LocalHost`]: a host loop on its own thread, for a program that embeds
//! the simulation side next to a view (the editor).
//!
//! The thread owns everything that simulates or edits: the `EditorDoc`, the
//! play session, the [`ErpServer`] with its proposals and activity log.
//! Nothing of it is shared with the caller: the view talks to it through a
//! [`LocalConnector`] (in-process ERP connections and frame streams) and,
//! if the server listens, through its socket like any other client.
//!
//! If the thread panics, the panic is caught: the connections see the host
//! go away, [`LocalHost::stopped_reason`] says why, and the program lives on.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use orr_edit::EditorDoc;
use orr_sim::Game;

use crate::host::Host;
use crate::link::LocalConnector;
use crate::server::{ErpServer, ServerConfig};

/// How often an idle host thread looks at its stop flag at least.
const IDLE: Duration = Duration::from_millis(1);

/// A host on its own thread (see the module docs).
pub struct LocalHost {
    connector: LocalConnector,
    url: Option<String>,
    stop: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

fn panic_text(p: &(dyn Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "the host thread panicked".to_string()
    }
}

impl LocalHost {
    /// Starts the thread. `setup` runs on it and builds the document and the
    /// server settings; it may fail (`Err` is returned here). The server
    /// starts from the settings, listening on `cfg.bind` only if `cfg.listen`.
    pub fn spawn<G: Game + 'static>(setup: impl FnOnce() -> Result<(EditorDoc, ServerConfig), String> + Send + 'static) -> Result<LocalHost, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let reason = Arc::new(Mutex::new(None::<String>));
        let (ready_tx, ready_rx) = channel::<Result<(LocalConnector, Option<String>), String>>();
        let (s, r) = (stop.clone(), reason.clone());
        let thread = std::thread::Builder::new()
            .name("orr-host".to_string())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let (doc, cfg) = match setup() {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let server = match ErpServer::start(cfg) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("ERP: {e}")));
                            return;
                        }
                    };
                    let url = server.is_listening().then(|| server.url());
                    let _ = ready_tx.send(Ok((server.connector(), url)));
                    let mut host = Host::<G>::new(doc, server);
                    host.run(&s, IDLE);
                }));
                if let Err(p) = outcome {
                    *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(panic_text(p.as_ref()));
                }
            })
            .map_err(|e| format!("cannot start the host thread: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Ok((connector, url))) => Ok(LocalHost { connector, url, stop, reason, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                // The thread died before it answered (setup panicked).
                let why = thread.join().map_err(|p| panic_text(p.as_ref())).err();
                let why = why.or_else(|| reason.lock().unwrap_or_else(|e| e.into_inner()).clone());
                Err(why.map_or_else(|| "the host did not start".to_string(), |w| format!("the host thread panicked while starting: {w}")))
            }
        }
    }

    /// Makes in-process connections to the host.
    pub fn connector(&self) -> LocalConnector {
        self.connector.clone()
    }

    /// `ws://host:port` if the server listens.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// True while the thread runs.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Why the host is gone: the panic message, or `None` if it still runs.
    /// Waits up to `wait` for a thread that is on its way out (a panic
    /// closes the connections a moment before the thread has recorded why).
    pub fn stopped_reason(&self, wait: Duration) -> Option<String> {
        let end = std::time::Instant::now() + wait;
        while self.is_running() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
        if self.is_running() {
            return None;
        }
        let why = self.reason.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Some(why.unwrap_or_else(|| "the host thread ended".to_string()))
    }
}

impl Drop for LocalHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
