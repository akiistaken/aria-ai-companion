//! File logging to logs/aria.log, with the app running windowless under release.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

struct Logger {
    file: Mutex<Option<File>>,
    debug: bool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

fn logger() -> &'static Logger {
    LOGGER.get_or_init(|| {
        let dir = crate::config::logs_dir();
        let _ = std::fs::create_dir_all(&dir);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("aria.log"))
            .ok();
        Logger {
            file: Mutex::new(file),
            debug: matches!(std::env::args().nth(1).as_deref(), Some("--debug")),
        }
    })
}

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        self.debug || meta.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} | {:<7} | {} | {}\n",
            crate::util::now_stamp("%Y-%m-%d %H:%M:%S"),
            record.level().as_str(),
            record.target(),
            record.args()
        );
        if let Some(file) = self
            .file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Some(file) = self
            .file
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = file.flush();
        }
    }
}

pub fn init() {
    let _ = log::set_logger(logger());
    log::set_max_level(if logger().debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
    log::info!("Aria starting | pid={}", std::process::id());
}

pub fn open_log_file() -> PathBuf {
    let dir = crate::config::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("aria.log");
    let _ = File::options().create(true).append(true).open(&path);
    let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
    path
}
