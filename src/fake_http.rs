//! A stand-in HTTP server for the tests of what talks to the platform, a
//! registry or a dashboard: each request kept, each answered as the test
//! says.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[derive(Debug, Clone)]
pub struct Asked {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Asked {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }

    /// A form's or a query's field, decoded.
    pub fn field(&self, name: &str) -> Option<String> {
        let source = if self.method == "GET" {
            self.path
                .split_once('?')
                .map(|(_, q)| q.to_string())
                .unwrap_or_default()
        } else {
            self.text()
        };
        source.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| decoded(value))
        })
    }
}

fn decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, value: serde_json::Value) -> Reply {
        Reply {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: value.to_string().into_bytes(),
        }
    }

    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>) -> Reply {
        Reply {
            status,
            headers: vec![("content-type".into(), content_type.into())],
            body,
        }
    }

    pub fn redirect(location: &str) -> Reply {
        Reply {
            status: 302,
            headers: vec![("location".into(), location.into())],
            body: Vec::new(),
        }
    }

    pub fn with(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub type Kept = Arc<Mutex<Vec<Asked>>>;

/// Serve on loopback until the test ends: its address, and every request.
pub async fn serve<F>(answer: F) -> (String, Kept)
where
    F: Fn(&Asked) -> Reply + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let kept: Kept = Arc::new(Mutex::new(Vec::new()));
    let held = kept.clone();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let held = held.clone();
            let answer = answer.clone();
            tokio::spawn(async move {
                let mut read = Vec::new();
                let mut chunk = [0u8; 16384];
                let (head_end, length) = loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    read.extend_from_slice(&chunk[..n]);
                    if let Some(end) = read.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&read[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while read.len() < head_end + length {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    read.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&read[..head_end]).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split(' ');
                let method = first.next().unwrap_or_default().to_string();
                let path = first.next().unwrap_or_default().to_string();
                let headers = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                let asked = Asked {
                    method,
                    path,
                    headers,
                    body: read[head_end..].to_vec(),
                };
                let reply = answer(&asked);
                held.lock().unwrap().push(asked);
                let mut out = format!("HTTP/1.1 {} X\r\n", reply.status);
                for (name, value) in &reply.headers {
                    out.push_str(&format!("{name}: {value}\r\n"));
                }
                out.push_str(&format!(
                    "content-length: {}\r\nconnection: close\r\n\r\n",
                    reply.body.len()
                ));
                let mut bytes = out.into_bytes();
                bytes.extend_from_slice(&reply.body);
                let _ = stream.write_all(&bytes).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (address, kept)
}

/// A directory of its own, removed when the test is done with it.
pub struct Scratch(pub std::path::PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Scratch {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("meridian-{label}-{unique}"));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
