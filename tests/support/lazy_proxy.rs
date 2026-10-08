//! Supported wire6 capability and lost-reply emulator. No historical binary claim.
use herdr_threads::daemon::{
    ownership::{read_descriptor, read_existing_namespace},
    paths::InstancePaths,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
// Single joined worker; no unowned per-connection threads. This is an explicit
// capability emulator, forwarding actual daemon decisions and recorded wire frames.
pub(crate) struct Proxy {
    pub(crate) mode: Arc<AtomicU8>,
    seen: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
    real: PathBuf,
    descriptor: PathBuf,
    saved: Vec<u8>,
}
pub(crate) fn frame(s: &mut UnixStream) -> Option<Vec<u8>> {
    let mut n = [0; 4];
    s.read_exact(&mut n).ok()?;
    let mut b = vec![0; u32::from_be_bytes(n) as usize];
    s.read_exact(&mut b).ok()?;
    Some(b)
}
pub(crate) fn write_frame(s: &mut UnixStream, b: &[u8]) {
    s.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
    s.write_all(b).unwrap();
}
impl Proxy {
    pub(crate) fn new(p: InstancePaths, private_root: &Path) -> Self {
        let instance = read_existing_namespace(&p).unwrap().unwrap();
        read_descriptor(&p, instance).unwrap();
        let socket = p.socket_path;
        let real = private_root.join("real.sock");
        fs::rename(&socket, &real).unwrap();
        let l = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        l.set_nonblocking(true).unwrap();
        let descriptor = p.descriptor_path;
        let saved = fs::read(&descriptor).unwrap();
        let mut v: Value = serde_json::from_slice(&saved).unwrap();
        let m = fs::metadata(&socket).unwrap();
        v["socket_device"] = json!(m.dev());
        v["socket_inode"] = json!(m.ino());
        fs::write(&descriptor, serde_json::to_vec(&v).unwrap()).unwrap();
        let mode = Arc::new(AtomicU8::new(0));
        let seen = Arc::new(Mutex::new(vec![]));
        let stop = Arc::new(AtomicBool::new(false));
        let (mode_c, seen_c, stop_c, real_c) =
            (mode.clone(), seen.clone(), stop.clone(), real.clone());
        let worker = std::thread::spawn(move || {
            while !stop_c.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((mut s, _)) => {
                        if s.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
                        let Some(b) = frame(&mut s) else { continue };
                        let q: Value = serde_json::from_slice(&b).unwrap();
                        seen_c.lock().unwrap().push(q.clone());
                        let mut d = UnixStream::connect(&real_c).unwrap();
                        d.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                        write_frame(&mut d, &b);
                        let Some(reply) = frame(&mut d) else { continue };
                        let mode = mode_c.load(Ordering::SeqCst);
                        if mode == 1 && q["command"]["kind"] == "capabilities" {
                            let mut v: Value = serde_json::from_slice(&reply).unwrap();
                            let caps = v["result"]["Ok"]["data"]["capabilities"]
                                .as_array_mut()
                                .expect("actual capability shape");
                            caps.retain(|c| {
                                !matches!(
                                    c.as_str(),
                                    Some(
                                        "send.lazy_v1"
                                            | "inbox.batch_v2"
                                            | "messages.delivery_modes_v1"
                                    )
                                )
                            });
                            write_frame(&mut s, &serde_json::to_vec(&v).unwrap());
                        } else if mode == 2 && q["command"]["kind"] == "complete_inbox_delivery" { // canonical commit, lost reply
                        } else {
                            write_frame(&mut s, &reply);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("proxy {e}"),
                }
            }
        });
        Self {
            mode,
            seen,
            stop,
            worker: Some(worker),
            socket,
            real,
            descriptor,
            saved,
        }
    }
    pub(crate) fn requests(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
        fs::remove_file(&self.socket).unwrap();
        fs::rename(&self.real, &self.socket).unwrap();
        fs::write(&self.descriptor, &self.saved).unwrap();
    }
}
