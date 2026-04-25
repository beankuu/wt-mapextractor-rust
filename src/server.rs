use std::fs;
use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use tiny_http::{Header, Response, Server, StatusCode};

fn can_gzip(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or_default(),
        "html" | "js" | "css" | "json" | "svg" | "txt" | "md"
    )
}

fn client_accepts_gzip(request: &tiny_http::Request) -> bool {
    request.headers().iter().any(|h| {
        h.field.equiv("Accept-Encoding")
            && h.value
                .as_str()
                .split(',')
                .any(|enc| enc.trim().starts_with("gzip"))
    })
}

fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

pub fn serve(root: &Path, addr: &str) -> Result<()> {
    let server = Server::http(addr).map_err(|e| anyhow!("Failed to bind {addr}: {e}"))?;
    println!("Serving {} at http://{addr}", root.display());
    println!("Type 'quit' (or 'exit'/'q') then Enter to stop serving.");

    let stop_flag = Arc::new(AtomicBool::new(false));
    {
        let stop_flag = Arc::clone(&stop_flag);
        thread::spawn(move || {
            let stdin = io::stdin();
            let mut locked = stdin.lock();
            let mut line = String::new();
            loop {
                line.clear();
                if locked.read_line(&mut line).is_err() {
                    break;
                }
                let cmd = line.trim().to_ascii_lowercase();
                if cmd == "quit" || cmd == "exit" || cmd == "q" {
                    stop_flag.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });
    }

    while !stop_flag.load(Ordering::SeqCst) {
        let request = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(e) => return Err(anyhow!("Server loop failed: {e}")),
        };
        let req_url = request.url().to_string();
        let path_only = req_url.split('?').next().unwrap_or("/");
        let url = path_only.trim_start_matches('/');

        if url == "favicon.ico" {
            let _ = request.respond(Response::empty(StatusCode(204)));
            continue;
        }

        let rel = if url.is_empty() { "src/index.html" } else { url };
        let mut path: PathBuf = root.join(rel);

        // If path doesn't exist, try "src/" as fallback
        if !path.is_file() {
            let src_path = root.join("src").join(url);
            if src_path.is_file() {
                path = src_path;
            }
        }

        let response = if path.is_file() {
            let mut bytes = Vec::new();
            match fs::File::open(&path).and_then(|mut f| f.read_to_end(&mut bytes).map(|_| ())) {
                Ok(_) => {
                    let use_gzip = bytes.len() > 512 && can_gzip(&path) && client_accepts_gzip(&request);
                    let (payload, is_gzipped) = if use_gzip {
                        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
                        if std::io::Write::write_all(&mut enc, &bytes).is_ok() {
                            match enc.finish() {
                                Ok(gz) if gz.len() + 24 < bytes.len() => (gz, true),
                                _ => (bytes, false),
                            }
                        } else {
                            (bytes, false)
                        }
                    } else {
                        (bytes, false)
                    };

                    let mut resp = Response::from_data(payload);
                    if let Ok(h) = Header::from_bytes(&b"Content-Type"[..], guess_mime(&path).as_bytes()) {
                        resp = resp.with_header(h);
                    }
                    if is_gzipped {
                        if let Ok(h) = Header::from_bytes(&b"Content-Encoding"[..], b"gzip") {
                            resp = resp.with_header(h);
                        }
                        if let Ok(h) = Header::from_bytes(&b"Vary"[..], b"Accept-Encoding") {
                            resp = resp.with_header(h);
                        }
                    }
                    if let Ok(h) = Header::from_bytes(&b"Cache-Control"[..], b"no-store, no-cache, must-revalidate, max-age=0") {
                        resp = resp.with_header(h);
                    }
                    if let Ok(h) = Header::from_bytes(&b"Pragma"[..], b"no-cache") {
                        resp = resp.with_header(h);
                    }
                    if let Ok(h) = Header::from_bytes(&b"Expires"[..], b"0") {
                        resp = resp.with_header(h);
                    }
                    resp
                }
                Err(_) => Response::from_string("Failed to read file")
                    .with_status_code(StatusCode(500)),
            }
        } else {
            Response::from_string("Not found").with_status_code(StatusCode(404))
        };

        let _ = request.respond(response);
    }

    println!("Server stopped.");

    Ok(())
}
