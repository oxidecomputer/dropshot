// Copyright 2020 Oxide Computer Company
//! Provides basic facilities for configuring logging and creating loggers, all
//! using Slog.  None of these facilities are required for this crate, but
//! they're provided because they're commonly wanted by consumers of this crate.

use camino::Utf8PathBuf;
use serde::Deserialize;
use serde::Serialize;
use slog::Drain;
use slog::Level;
use slog::Logger;
use std::fs::OpenOptions;
use std::io::LineWriter;
use std::io::Write;
use std::{io, path::Path};

/// Represents the logging configuration for a server.  This is expected to be a
/// top-level block in a TOML config file, although that's not required.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "mode")]
pub enum ConfigLogging {
    /// Pretty-printed output to stderr, assumed to support terminal escapes.
    StderrTerminal { level: ConfigLoggingLevel },
    /// Bunyan-formatted output to a specified file.
    File {
        level: ConfigLoggingLevel,
        path: Utf8PathBuf,
        if_exists: ConfigLoggingIfExists,
    },
}

/// Log messages have a level that's used for filtering in the usual way.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigLoggingLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl From<&ConfigLoggingLevel> for Level {
    fn from(config_level: &ConfigLoggingLevel) -> Level {
        match config_level {
            ConfigLoggingLevel::Trace => Level::Trace,
            ConfigLoggingLevel::Debug => Level::Debug,
            ConfigLoggingLevel::Info => Level::Info,
            ConfigLoggingLevel::Warn => Level::Warning,
            ConfigLoggingLevel::Error => Level::Error,
            ConfigLoggingLevel::Critical => Level::Critical,
        }
    }
}

/// Specifies the behavior when logging to a file that already exists.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigLoggingIfExists {
    /// Fail to create the log
    Fail,
    /// Truncate the existing file
    Truncate,
    /// Append to the existing file
    Append,
}

impl ConfigLogging {
    /// Create a root logger based on the requested configuration.
    pub fn to_logger<S: AsRef<str>>(
        &self,
        log_name: S,
    ) -> Result<Logger, io::Error> {
        match self {
            ConfigLogging::StderrTerminal { level } => {
                let decorator = slog_term::TermDecorator::new().build();
                let drain =
                    slog_term::FullFormat::new(decorator).build().fuse();
                Ok(async_root_logger(level, drain))
            }

            ConfigLogging::File { level, path, if_exists } => {
                let mut open_options = std::fs::OpenOptions::new();
                open_options.write(true);
                open_options.create(true);

                match if_exists {
                    ConfigLoggingIfExists::Fail => {
                        open_options.create_new(true);
                    }
                    ConfigLoggingIfExists::Append => {
                        open_options.append(true);
                    }
                    ConfigLoggingIfExists::Truncate => {
                        open_options.truncate(true);
                    }
                }

                let drain = log_drain_for_file(
                    &open_options,
                    Path::new(path),
                    log_name.as_ref().to_string(),
                )?;
                let logger = async_root_logger(level, drain);

                // Record a message to the stderr so that a reader who doesn't
                // already know how logging is configured knows where the rest
                // of the log messages went.
                //
                // We don't want to use `eprintln!`, because it panics if stderr
                // isn't writeable (e.g., if stderr has been redirected to a
                // file on a full disk). Our options seem to be:
                //
                // a) Return an error if we fail to emit this message
                // b) Silently swallow errors
                // c) If an error occurs, try to relay that fact
                //
                // (a) doesn't seem great, because it's possible we're able to
                // run just fine (and possibly even use the logger we're about
                // to create, as we don't know that it will suffer the same fate
                // that writing to stderr does). (b) is defensible since this is
                // just an informational message. We go with (c) and attempt to
                // leave a breadcrumb in the log itself.
                if let Err(err) = writeln!(
                    io::stderr(),
                    "note: configured to log to \"{path}\"",
                ) {
                    slog::warn!(
                        logger,
                        "failed to report log path on stderr";
                        "err" => %err,
                    );
                }

                Ok(logger)
            }
        }
    }
}

// TODO-hardening We use an async drain to take care of synchronization.  That's
// mainly because the other two documented options use a std::sync::Mutex, which
// is not futures-aware and is likely to foul up our executor.  However, we have
// not verified that the async implementation behaves reasonably under
// backpressure, and it definitely makes things harder to debug.
fn async_root_logger<T>(level: &ConfigLoggingLevel, drain: T) -> slog::Logger
where
    T: slog::Drain + Send + 'static,
    <T as slog::Drain>::Err: std::fmt::Debug,
{
    let level_drain = slog::LevelFilter(drain, Level::from(level)).fuse();
    let async_drain =
        slog_async::Async::new(level_drain).chan_size(1024).build().fuse();
    slog::Logger::root(async_drain, o!())
}

fn log_drain_for_file(
    open_options: &OpenOptions,
    path: &Path,
    log_name: String,
) -> Result<slog::Fuse<slog_json::Json<LineWriter<std::fs::File>>>, io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Buffer writes to the file around newlines to minimize syscalls.
    let file = LineWriter::new(open_options.open(path)?);

    // Using leak() here is dubious.  However, we really want the logger's name
    // to be dynamically generated from the test name.  Unfortunately, the
    // bunyan interface requires that it be a `&'static str`.  The correct
    // approach is to fix that interface.
    // TODO-cleanup
    let log_name_box = Box::new(log_name);
    let log_name_leaked = Box::leak(log_name_box);
    Ok(slog_bunyan::with_name(log_name_leaked, file).build().fuse())
}

#[cfg(test)]
pub mod test {
    use crate::test_util::read_bunyan_log;
    use crate::test_util::read_config;
    use crate::test_util::verify_bunyan_records;
    use crate::test_util::verify_bunyan_records_sequential;
    use crate::test_util::BunyanLogRecordSpec;
    use crate::ConfigLogging;
    use libc;
    use slog::Logger;
    use std::fs;
    use std::path::Path;
    use std::{io, path::PathBuf};

    /// Generates a temporary filesystem path unique for the given label.
    fn temp_path(label: &str) -> PathBuf {
        let arg0str = std::env::args().next().expect("expected process arg0");
        let arg0 = Path::new(&arg0str)
            .file_name()
            .expect("expected arg0 filename")
            .to_str()
            .expect("expected arg0 filename to be valid Unicode");
        let pid = std::process::id();
        let mut pathbuf = std::env::temp_dir();
        pathbuf.push(format!("{}.{}.{}", arg0, pid, label));
        pathbuf
    }

    /// Load a configuration and create a logger from it.
    pub fn read_config_and_create_logger(
        label: &str,
        contents: &str,
    ) -> Result<Logger, io::Error> {
        let config = read_config::<ConfigLogging>(label, contents).unwrap();
        let result = config.to_logger("test-logger");
        if let Err(ref error) = result {
            eprintln!("error message creating logger: {}", error);
        }
        result
    }

    // Bad value for "log_mode"

    #[test]
    fn test_config_bad_log_mode() {
        let bad_config = r##" mode = "bonkers" "##;
        let error = read_config::<ConfigLogging>("bad_log_mode", bad_config)
            .unwrap_err()
            .to_string();
        println!("error: {}", error);
        assert!(error.contains(
            "unknown variant `bonkers`, expected `stderr-terminal` or `file`"
        ));
    }

    // Bad "mode = stderr-terminal" config
    //
    // TODO-coverage: consider adding tests for all variants of missing or
    // invalid properties for all log modes

    #[test]
    fn test_config_bad_terminal_no_level() {
        let bad_config = r##" mode = "stderr-terminal" "##;
        let error =
            read_config::<ConfigLogging>("bad_terminal_no_level", bad_config)
                .unwrap_err()
                .to_string();
        assert_eq!(error.trim_end(), "missing field `level`");
    }

    #[test]
    fn test_config_bad_terminal_bad_level() {
        let bad_config = r##"
            mode = "stderr-terminal"
            level = "everything"
            "##;
        let error =
            read_config::<ConfigLogging>("bad_terminal_bad_level", bad_config)
                .unwrap_err()
                .to_string();
        assert_eq!(
            error.trim_end(),
            "unknown variant `everything`, expected one of `trace`, `debug`, \
             `info`, `warn`, `error`, `critical`"
        );
    }

    // Working "mode = stderr-terminal" config
    //
    // TODO-coverage: It would be nice to redirect our own stderr to a file (or
    // something else we can collect) and then use the logger that we get below.
    // Then we could verify that it contains the content we expect.
    // Unfortunately, while Rust has private functions to redirect stdout and
    // stderr, there's no exposed function for doing that, nor is there a way to
    // provide a specific stream to a terminal logger.  (We could always
    // implement our own.)
    #[test]
    fn test_config_stderr_terminal() {
        let config = r##"
            mode = "stderr-terminal"
            level = "warn"
        "##;
        let config =
            read_config::<ConfigLogging>("stderr-terminal", config).unwrap();
        config.to_logger("test-logger").unwrap();
    }

    // Bad "mode = file" configurations

    #[test]
    fn test_config_bad_file_no_file() {
        let bad_config = r##"
            mode = "file"
            level = "warn"
            "##;
        let error =
            read_config::<ConfigLogging>("bad_file_no_file", bad_config)
                .unwrap_err()
                .to_string();
        assert_eq!(error.trim_end(), "missing field `path`");
    }

    #[test]
    fn test_config_bad_file_no_level() {
        let bad_config = r##"
            mode = "file"
            path = "nonexistent"
            "##;
        let error =
            read_config::<ConfigLogging>("bad_file_no_level", bad_config)
                .unwrap_err()
                .to_string();
        assert_eq!(error.trim_end(), "missing field `level`");
    }

    /// `LogTest` and `LogTestCleanup` are used for the tests that create various
    /// files on the filesystem to commonize code and make sure everything gets
    /// cleaned up as expected.
    pub struct LogTest {
        directory: PathBuf,
        cleanup_list: Vec<LogTestCleanup>,
    }

    #[derive(Debug)]
    enum LogTestCleanup {
        Directory(PathBuf),
        File(PathBuf),
    }

    impl LogTest {
        /// The setup for a logger test creates a temporary directory with the
        /// given label and returns a `LogTest` with that directory in the
        /// cleanup list so that on teardown the temporary directory will be
        /// removed.  The temporary directory must be empty by the time the
        /// `LogTest` is torn down except for files and directories created with
        /// `will_create_dir()` and `will_create_file()`.
        pub fn setup(label: &str) -> LogTest {
            let directory_path = temp_path(label);

            if let Err(e) = fs::create_dir_all(&directory_path) {
                panic!(
                    "unexpected failure creating directories leading up to \
                     {}: {}",
                    directory_path.as_path().display(),
                    e
                );
            }

            LogTest {
                directory: directory_path.clone(),
                cleanup_list: vec![LogTestCleanup::Directory(directory_path)],
            }
        }

        /// Records that the caller intends to create a directory with relative
        /// path "path" underneath the root directory for this log test. Returns
        /// the path to this directory. This directory will be removed during
        /// teardown. Directories and files must be recorded in the order they
        /// would be created so that the order can be reversed at teardown
        /// (without needing any kind of recursive removal).
        pub fn will_create_dir(&mut self, path: &str) -> PathBuf {
            let mut pathbuf = self.directory.clone();
            pathbuf.push(path);
            self.cleanup_list.push(LogTestCleanup::Directory(pathbuf.clone()));
            pathbuf
        }

        /// Records that the caller intends to create a file with relative path
        /// "path" underneath the root directory for this log test. Returns a
        /// the path to this file. This file will be removed during teardown.
        /// Directories and files must be recorded in the order they would be
        /// created so that the order can be reversed at teardown (without
        /// needing any kind of recursive removal).
        pub fn will_create_file(&mut self, path: &str) -> PathBuf {
            let mut pathbuf = self.directory.clone();
            pathbuf.push(path);
            self.cleanup_list.push(LogTestCleanup::File(pathbuf.clone()));
            pathbuf
        }
    }

    impl Drop for LogTest {
        fn drop(&mut self) {
            for path in self.cleanup_list.iter().rev() {
                let maybe_error = match path {
                    LogTestCleanup::Directory(p) => fs::remove_dir(p),
                    LogTestCleanup::File(p) => fs::remove_file(p),
                };

                if let Err(e) = maybe_error {
                    panic!("unexpected failure removing {:?}", e);
                }
            }
        }
    }

    #[test]
    fn test_config_bad_file_bad_path_type() {
        // We create a path as a directory so that when we subsequently try to
        // use it a file, we won't be able to.
        let mut logtest = LogTest::setup("bad_file_bad_path_type_dir");
        let path = logtest.will_create_dir("log_file_as_dir");
        fs::create_dir(&path).unwrap();

        // Windows paths need to have \ turned into \\
        let escaped_path =
            path.display().to_string().escape_default().to_string();

        let bad_config = format!(
            r#"
            mode = "file"
            level = "warn"
            if_exists = "append"
            path = "{}"
            "#,
            escaped_path
        );

        let error = read_config_and_create_logger(
            "bad_file_bad_path_type",
            &bad_config,
        )
        .unwrap_err();

        if cfg!(windows) {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        } else {
            // We expect ErrorKind::IsADirectory which is nightly-only atm
            // assert_eq!(error.kind(), std::io::ErrorKind::IsADirectory);
            assert_eq!(error.raw_os_error(), Some(libc::EISDIR));
        }
    }

    #[test]
    fn test_config_bad_file_path_exists_fail() {
        let mut logtest = LogTest::setup("bad_file_path_exists_fail_dir");
        let logpath = logtest.will_create_file("log.out");
        fs::write(&logpath, "").expect("writing empty file");

        // Windows paths need to have \ turned into \\
        let escaped_path =
            logpath.display().to_string().escape_default().to_string();

        let bad_config = format!(
            r#"
            mode = "file"
            level = "warn"
            if_exists = "fail"
            path = "{}"
            "#,
            escaped_path
        );

        let error = read_config_and_create_logger(
            "bad_file_bad_path_exists_fail",
            &bad_config,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    }

    // Working "mode = file" configuration.  The following test exercises
    // successful file-based configurations for all three values of "if_exists",
    // different log levels, and the bunyan log format.

    #[test]
    fn test_config_file() {
        let mut logtest = LogTest::setup("file_dir");
        let logpath = logtest.will_create_file("log.out");
        let time_before = chrono::offset::Utc::now();

        // Windows paths need to have \ turned into \\
        let escaped_path =
            logpath.display().to_string().escape_default().to_string();

        // The first attempt should succeed.  The log file doesn't exist yet.
        let config = format!(
            r#"
            mode = "file"
            level = "warn"
            if_exists = "fail"
            path = "{}"
            "#,
            escaped_path
        );

        {
            // Construct the logger in a block so that it's flushed by the time
            // we proceed.
            let log = read_config_and_create_logger("file", &config).unwrap();
            debug!(log, "message1_debug");
            warn!(log, "message1_warn");
            error!(log, "message1_error");
        }

        // Try again with if_exists = "append".  This should also work.
        let config = format!(
            r#"
            mode = "file"
            level = "warn"
            if_exists = "append"
            path = "{}"
            "#,
            escaped_path
        );

        {
            // See above.
            let log = read_config_and_create_logger("file", &config).unwrap();
            warn!(log, "message2");
        }

        let time_after = chrono::offset::Utc::now();
        let log_records = read_bunyan_log(&logpath);
        let expected_hostname = hostname::get().unwrap().into_string().unwrap();
        verify_bunyan_records(
            log_records.iter(),
            &BunyanLogRecordSpec {
                name: Some("test-logger".to_string()),
                hostname: Some(expected_hostname.clone()),
                v: Some(0),
                pid: Some(std::process::id()),
            },
        );
        verify_bunyan_records_sequential(
            log_records.iter(),
            Some(&time_before),
            Some(&time_after),
        );

        assert_eq!(log_records.len(), 3);
        assert_eq!(log_records[0].msg, "message1_warn");
        assert_eq!(log_records[1].msg, "message1_error");
        assert_eq!(log_records[2].msg, "message2");

        // Try again with if_exists = "truncate".  This should also work, but
        // remove the contents that's already there.
        let time_before = time_after;
        let time_after = chrono::offset::Utc::now();
        let config = format!(
            r#"
            mode = "file"
            level = "trace"
            if_exists = "truncate"
            path = "{}"
            "#,
            escaped_path
        );

        {
            // See above.
            let log = read_config_and_create_logger("file", &config).unwrap();
            debug!(log, "message3_debug");
            warn!(log, "message3_warn");
            error!(log, "message3_error");
        }

        let log_records = read_bunyan_log(&logpath);
        verify_bunyan_records(
            log_records.iter(),
            &BunyanLogRecordSpec {
                name: Some("test-logger".to_string()),
                hostname: Some(expected_hostname),
                v: Some(0),
                pid: Some(std::process::id()),
            },
        );
        verify_bunyan_records_sequential(
            log_records.iter(),
            Some(&time_before),
            Some(&time_after),
        );
        assert_eq!(log_records.len(), 3);
        assert_eq!(log_records[0].msg, "message3_debug");
        assert_eq!(log_records[1].msg, "message3_warn");
        assert_eq!(log_records[2].msg, "message3_error");
    }
}
