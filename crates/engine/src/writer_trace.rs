use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Step {
    operation: &'static str,
    since: Instant,
    index: usize,
    size: u64,
    read: u64,
}

/// Sample independently of the writer, so a synchronous call that never
/// returns still leaves a diagnosis. Keep this outside the job manifest.
pub(crate) struct WriterTrace {
    step: Arc<Mutex<Step>>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WriterTrace {
    pub fn start(dir: &Path, total: usize, level: i32, serial: bool) -> Self {
        let step = Arc::new(Mutex::new(Step {
            operation: "opening archive",
            since: Instant::now(),
            index: 0,
            size: 0,
            read: 0,
        }));
        let (stop, receiver) = mpsc::channel();
        let worker_step = step.clone();
        let dir = dir.to_owned();
        let thread = thread::Builder::new()
            .name("zim-writer-trace".into())
            .spawn(move || sample(dir, total, level, serial, worker_step, receiver))
            .ok();
        Self { step, stop, thread }
    }

    pub fn at(&self, operation: &'static str, index: usize, size: u64, read: u64) {
        *self.step.lock().unwrap() = Step {
            operation,
            since: Instant::now(),
            index,
            size,
            read,
        };
    }
}

impl Drop for WriterTrace {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn sample(
    dir: PathBuf,
    total: usize,
    level: i32,
    serial: bool,
    state: Arc<Mutex<Step>>,
    stop: mpsc::Receiver<()>,
) {
    let mut history = VecDeque::new();
    let partial = dir.join("archive.partial.zim");
    let log = dir.join("archive-writer.log");
    let mut stopped = false;
    loop {
        let step = state.lock().unwrap().clone();
        let output = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        history.push_back(format!(
            "{} {} file_index={}/{} input_bytes={}/{} partial_bytes={} operation_elapsed_ms={}",
            crate::now(),
            step.operation,
            step.index,
            total,
            step.read,
            step.size,
            output,
            step.since.elapsed().as_millis()
        ));
        if history.len() > 48 {
            history.pop_front();
        }
        let write = || -> std::io::Result<()> {
            let mut file = atomic_write_file::AtomicWriteFile::open(&log)?;
            writeln!(
                file,
                "serial={serial} zstd_level={level} (file_index is zero-based)"
            )?;
            for line in &history {
                writeln!(file, "{line}")?;
            }
            file.commit()
        };
        let _ = write();
        if stopped {
            break;
        }
        stopped = !matches!(
            stop.recv_timeout(Duration::from_secs(3)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
    }
}
