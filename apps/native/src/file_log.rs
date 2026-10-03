//! Retained release log, like the baseline log plugin: warnings globally and
//! info for dbunk crates, mirrored to stderr. Files live under
//! ~/Library/Logs because profiles admit only their own files. Each profile
//! gets its own file; size is bounded with one rotated predecessor and lines
//! are redacted and clipped before they reach disk.
use std::{
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const FILE_BYTES: u64 = 1024 * 1024;
const LINE_BYTES: usize = 8 * 1024;
const DIRECTORY: &str = "dbunk Native";

struct State {
    path: PathBuf,
    file: Option<File>,
    size: u64,
}
struct FileLog(Mutex<State>);

fn enabled(metadata: &log::Metadata) -> bool {
    let crate_level = if cfg!(debug_assertions) {
        log::Level::Debug
    } else {
        log::Level::Info
    };
    let target = metadata.target();
    let ours = target.starts_with("dbunk") || target.starts_with("dbunk_native");
    metadata.level() <= if ours { crate_level } else { log::Level::Warn }
}

/// Removes passwords from URIs and `password=` pairs, then clips to a
/// character boundary. Logged text is diagnostic, never a secret channel.
pub fn redact(line: &str) -> String {
    let mut out = String::with_capacity(line.len().min(LINE_BYTES));
    let mut rest = line;
    while let Some(scheme) = rest.find("://") {
        let (head, tail) = rest.split_at(scheme + 3);
        out.push_str(head);
        let authority_end = tail.find(['/', ' ', '?', '#']).unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match (authority.rfind('@'), authority.find(':')) {
            (Some(at), Some(colon)) if colon < at => {
                out.push_str(&authority[..colon]);
                out.push_str(":***");
                out.push_str(&authority[at..]);
            }
            _ => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    let lower = out.to_ascii_lowercase();
    let mut redacted = String::with_capacity(out.len());
    let mut cursor = 0;
    let mut search = 0;
    while let Some(found) = ["password=", "passwd=", "pwd="]
        .iter()
        .filter_map(|key| lower[search..].find(key).map(|at| (search + at, key.len())))
        .min()
    {
        let value = found.0 + found.1;
        let end = out[value..]
            .find([' ', '&', ';', ',', '\n'])
            .map_or(out.len(), |at| value + at);
        redacted.push_str(&out[cursor..value]);
        redacted.push_str("***");
        cursor = end;
        search = end;
    }
    redacted.push_str(&out[cursor..]);
    if redacted.len() > LINE_BYTES {
        let mut end = LINE_BYTES;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        redacted.truncate(end);
        redacted.push('…');
    }
    redacted
}

impl State {
    fn write(&mut self, line: &str) {
        let bytes = line.len() as u64 + 1;
        if self.size + bytes > FILE_BYTES {
            self.file = None;
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            self.size = 0;
        }
        if self.file.is_none() {
            self.file = open(&self.path).ok();
            self.size = self
                .file
                .as_ref()
                .and_then(|file| file.metadata().ok())
                .map_or(0, |metadata| metadata.len());
        }
        if let Some(file) = &mut self.file
            && writeln!(file, "{line}").is_ok()
        {
            self.size += bytes;
        }
    }
}
fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

impl log::Log for FileLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        enabled(metadata)
    }
    fn log(&self, record: &log::Record) {
        if !enabled(record.metadata()) {
            return;
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let line = redact(&format!(
            "{}.{:03} {} {}: {}",
            at.as_secs(),
            at.subsec_millis(),
            record.level(),
            record.target(),
            record.args()
        ));
        eprintln!("{line}");
        if let Ok(mut state) = self.0.lock() {
            state.write(&line);
        }
    }
    fn flush(&self) {
        if let Ok(mut state) = self.0.lock()
            && let Some(file) = &mut state.file
        {
            let _ = file.flush();
        }
    }
}

/// Log file for one profile path; the hash keeps concurrent profiles apart.
pub fn path_for(logs: &Path, profile: &Path) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile.hash(&mut hasher);
    logs.join(format!("dbunk-native-{:016x}.log", hasher.finish()))
}

/// Installs the logger. Failure leaves the app running with stderr only and
/// returns the reason for the caller to report.
pub fn install(profile: &Path) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unset; file logging disabled")?;
    let logs = PathBuf::from(home).join("Library/Logs").join(DIRECTORY);
    std::fs::create_dir_all(&logs).map_err(|error| format!("Log directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o700));
    }
    let path = path_for(&logs, profile);
    let file = open(&path).map_err(|error| format!("Log file: {error}"))?;
    let size = file.metadata().map_or(0, |metadata| metadata.len());
    log::set_boxed_logger(Box::new(FileLog(Mutex::new(State {
        path: path.clone(),
        file: Some(file),
        size,
    }))))
    .map_err(|error| error.to_string())?;
    log::set_max_level(if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_and_lines_clipped_on_char_boundaries() {
        assert_eq!(
            redact("dial postgres://app:s3cr@t@db:5432/x?sslmode=require"),
            "dial postgres://app:***@db:5432/x?sslmode=require"
        );
        assert_eq!(
            redact("host=db user=app PASSWORD=hunter2 dbname=x pwd=a&b"),
            "host=db user=app PASSWORD=*** dbname=x pwd=***&b"
        );
        assert_eq!(
            redact("postgres://db/x no secret"),
            "postgres://db/x no secret"
        );
        let long = "字".repeat(LINE_BYTES);
        let clipped = redact(&long);
        assert!(clipped.len() <= LINE_BYTES + '…'.len_utf8());
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn files_rotate_once_and_stay_bounded() {
        let directory =
            std::env::temp_dir().join(format!("dbunk-native-log-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = path_for(&directory, Path::new("/owned/profile"));
        assert_ne!(path, path_for(&directory, Path::new("/owned/other")));
        let mut state = State {
            path: path.clone(),
            file: None,
            size: 0,
        };
        let line = "x".repeat(1000);
        for _ in 0..3000 {
            state.write(&line);
        }
        let current = std::fs::metadata(&path).unwrap().len();
        let rotated = std::fs::metadata(path.with_extension("log.1"))
            .unwrap()
            .len();
        assert!(current <= FILE_BYTES && rotated <= FILE_BYTES);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
