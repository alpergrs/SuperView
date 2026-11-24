//! One window per user. A second launch hands its file to the running window and exits.

use crate::OpenRequest;
use eframe::egui;
use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{mpsc::Sender, Arc, OnceLock};

/// Lets the listener thread wake the UI once the window exists.
#[derive(Clone, Default)]
pub struct Waker(Arc<OnceLock<egui::Context>>);

impl Waker {
    pub fn set(&self, ctx: egui::Context) {
        let _ = self.0.set(ctx);
    }

    fn wake(&self) {
        if let Some(ctx) = self.0.get() {
            ctx.request_repaint();
        }
    }
}

fn socket_name() -> String {
    // Per user, so two people on one machine each get their own window.
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let user: String = user.chars().filter(|c| c.is_alphanumeric()).collect();
    format!("superview-{user}.sock")
}

fn encode(request: Option<&OpenRequest>) -> String {
    match request {
        Some(r) => {
            let kind = if r.slideshow { 'S' } else if r.recursive { 'R' } else { 'F' };
            format!("{kind}{}\n", r.path.display())
        }
        None => "N\n".to_string(),
    }
}

fn decode(line: &str) -> Option<Option<OpenRequest>> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut chars = line.chars();
    match chars.next()? {
        'N' => Some(None),
        kind @ ('R' | 'F' | 'S') if !chars.as_str().is_empty() => Some(Some(OpenRequest {
            path: PathBuf::from(chars.as_str()),
            recursive: kind == 'R',
            slideshow: kind == 'S',
        })),
        _ => None,
    }
}

/// Returns `false` when another instance accepted the request and this process should
/// exit. Otherwise this process becomes the instance and forwards later requests to `tx`
/// (`None` means "just come to the front").
pub fn claim(request: Option<&OpenRequest>, tx: Sender<Option<OpenRequest>>, waker: Waker) -> bool {
    let name = socket_name();
    if let Ok(ns) = name.as_str().to_ns_name::<GenericNamespaced>() {
        if let Ok(mut conn) = Stream::connect(ns) {
            if conn.write_all(encode(request).as_bytes()).is_ok() {
                let _ = conn.flush();
                return false;
            }
        }
    }
    let Ok(ns) = name.to_ns_name::<GenericNamespaced>() else { return true };
    let Ok(listener) = ListenerOptions::new().name(ns).create_sync() else { return true };
    std::thread::spawn(move || {
        for conn in listener.incoming().filter_map(Result::ok) {
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_err() {
                continue;
            }
            if let Some(request) = decode(&line) {
                if tx.send(request).is_err() {
                    break;
                }
                waker.wake();
            }
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let r = OpenRequest { path: PathBuf::from("C:\\Photos\\a b.jpg"), recursive: true, slideshow: false };
        assert_eq!(decode(&encode(Some(&r))), Some(Some(r)));
        let s = OpenRequest { path: PathBuf::from("C:\\Photos"), recursive: false, slideshow: true };
        assert_eq!(decode(&encode(Some(&s))), Some(Some(s)));
        assert_eq!(decode(&encode(None)), Some(None));
        assert_eq!(decode("garbage"), None);
        assert_eq!(decode("F\n"), None);
    }
}
