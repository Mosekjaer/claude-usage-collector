// Logs to stderr and to <config-dir>/collector.log (truncated at start).

use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Logger {
    file: Mutex<Option<File>>,
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= log::max_level()
    }
    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), r.level(), r.args());
        let _ = std::io::stderr().write_all(line.as_bytes());
        if let Ok(mut f) = self.file.lock() {
            if let Some(f) = f.as_mut() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }
    fn flush(&self) {}
}

/// `truncate`: the long-running service starts a fresh log; one-shot commands append.
pub fn init(path: &std::path::Path, debug: bool, truncate: bool) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = if truncate { File::create(path).ok() } else { std::fs::OpenOptions::new().append(true).create(true).open(path).ok() };
    let logger = Box::leak(Box::new(Logger { file: Mutex::new(file) }));
    let _ = log::set_logger(logger);
    log::set_max_level(if debug { LevelFilter::Debug } else { LevelFilter::Info });
    let _ = Level::Info;
}
