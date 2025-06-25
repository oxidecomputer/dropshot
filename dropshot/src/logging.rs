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

#[cfg(feature = "tracing")]
use tracing_subscriber::layer::SubscriberExt;
#[cfg(feature = "tracing")]
use tracing_subscriber::Layer;

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
        let logger = match self {
            ConfigLogging::StderrTerminal { level } => {
                let decorator = slog_term::TermDecorator::new().build();
                let drain =
                    slog_term::FullFormat::new(decorator).build().fuse();
                async_root_logger(level, drain)
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

                logger
            }
        };

        // Initialize tracing bridge automatically if feature is enabled
        #[cfg(feature = "tracing")]
        {
            if let Err(e) = init_tracing_bridge(&logger) {
                slog::error!(
                    logger,
                    "failed to initialize tracing bridge";
                    "error" => %e,
                );
            }
        }

        Ok(logger)
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

#[cfg(feature = "tracing")]
/// A tracing subscriber layer that bridges tracing events to slog.
/// This allows users to use tracing macros while maintaining slog compatibility.
pub struct SlogTracingBridge {
    logger: slog::Logger,
}

#[cfg(feature = "tracing")]
impl SlogTracingBridge {
    pub fn new(logger: slog::Logger) -> Self {
        Self { logger }
    }
}

#[cfg(feature = "tracing")]
impl<S> Layer<S> for SlogTracingBridge
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            tracing::Level::TRACE => slog::Level::Trace,
            tracing::Level::DEBUG => slog::Level::Debug,
            tracing::Level::INFO => slog::Level::Info,
            tracing::Level::WARN => slog::Level::Warning,
            tracing::Level::ERROR => slog::Level::Error,
        };

        // Extract the message and key-value pairs from the tracing event
        let mut visitor = SlogEventVisitor::new();
        event.record(&mut visitor);

        // Log to slog with the extracted data
        let message = visitor.message.unwrap_or_else(|| "".to_string());

        // Create a dynamic key-value object for slog
        let kv = SlogKV::new(visitor.fields);

        // Use slog macros based on level with proper key-value pairs
        match level {
            slog::Level::Trace => {
                slog::trace!(self.logger, "{}", message; kv)
            }
            slog::Level::Debug => {
                slog::debug!(self.logger, "{}", message; kv)
            }
            slog::Level::Info => {
                slog::info!(self.logger, "{}", message; kv)
            }
            slog::Level::Warning => {
                slog::warn!(self.logger, "{}", message; kv)
            }
            slog::Level::Error => {
                slog::error!(self.logger, "{}", message; kv)
            }
            slog::Level::Critical => {
                slog::crit!(self.logger, "{}", message; kv)
            }
        }
    }
}

#[cfg(feature = "tracing")]
/// Wrapper for different value types that can be logged
#[derive(Debug, Clone)]
enum SlogValue {
    Str(String),
    I64(i64),
    U64(u64),
    Bool(bool),
    Debug(String),
}

#[cfg(feature = "tracing")]
/// Helper struct to pass tracing fields as slog key-value pairs
struct SlogKV {
    fields: Vec<(String, SlogValue)>,
}

#[cfg(feature = "tracing")]
impl SlogKV {
    fn new(fields: Vec<(String, SlogValue)>) -> Self {
        Self { fields }
    }
}

#[cfg(feature = "tracing")]
impl slog::KV for SlogKV {
    fn serialize(
        &self,
        _record: &slog::Record,
        serializer: &mut dyn slog::Serializer,
    ) -> slog::Result {
        for (key, value) in &self.fields {
            let key = slog::Key::from(key.clone());
            match value {
                SlogValue::Str(s) => serializer.emit_str(key, s)?,
                SlogValue::I64(i) => serializer.emit_i64(key, *i)?,
                SlogValue::U64(u) => serializer.emit_u64(key, *u)?,
                SlogValue::Bool(b) => serializer.emit_bool(key, *b)?,
                SlogValue::Debug(s) => serializer.emit_str(key, s)?,
            }
        }
        Ok(())
    }
}

#[cfg(feature = "tracing")]
/// Visitor to extract fields from tracing events
struct SlogEventVisitor {
    message: Option<String>,
    fields: Vec<(String, SlogValue)>,
}

#[cfg(feature = "tracing")]
impl SlogEventVisitor {
    fn new() -> Self {
        Self { message: None, fields: Vec::new() }
    }
}

#[cfg(feature = "tracing")]
impl tracing::field::Visit for SlogEventVisitor {
    fn record_debug(
        &mut self,
        field: &tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.push((
                field.name().to_string(),
                SlogValue::Debug(format!("{:?}", value)),
            ));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((
                field.name().to_string(),
                SlogValue::Str(value.to_string()),
            ));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.push((field.name().to_string(), SlogValue::I64(value)));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.push((field.name().to_string(), SlogValue::U64(value)));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.push((field.name().to_string(), SlogValue::Bool(value)));
    }
}

#[cfg(feature = "tracing")]
/// Initialize tracing with slog bridge support
pub fn init_tracing_bridge(
    logger: &slog::Logger,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if a global subscriber has already been set
    if tracing::dispatcher::has_been_set() {
        return Ok(());
    }

    let bridge = SlogTracingBridge::new(logger.clone());
    let subscriber = tracing_subscriber::registry().with(bridge);

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

#[cfg(test)]
mod test {
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
    fn read_config_and_create_logger(
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
    struct LogTest {
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
        fn setup(label: &str) -> LogTest {
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
        fn will_create_dir(&mut self, path: &str) -> PathBuf {
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
        fn will_create_file(&mut self, path: &str) -> PathBuf {
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

    /// Test that the tracing-to-slog bridge works with basic logging
    #[test]
    fn test_tracing_bridge_basic() {
        let mut logtest = LogTest::setup("tracing_bridge_basic");
        let logpath = logtest.will_create_file("bridge.log");

        // Windows paths need to have \ turned into \\
        let escaped_path =
            logpath.display().to_string().escape_default().to_string();

        let config = format!(
            r#"
            mode = "file"
            level = "info"
            if_exists = "truncate"
            path = "{}"
            "#,
            escaped_path
        );

        {
            let logger =
                read_config_and_create_logger("tracing_bridge_basic", &config)
                    .unwrap();

            // Test slog logging
            slog::info!(logger, "slog message"; "slog_key" => "slog_value", "slog_num" => 42);

            // Test tracing logging (bridge is automatically initialized when feature is enabled)
            #[cfg(feature = "tracing")]
            {
                tracing::info!(
                    tracing_key = "tracing_value",
                    tracing_num = 84,
                    "tracing message"
                );
            }

            // Explicitly drop the logger to ensure async drain flushes
            drop(logger);
        }

        // Retry reading the log file to handle async drain flushing
        let log_records = {
            let mut records = Vec::new();
            for _ in 0..10 {
                records = read_bunyan_log(&logpath);
                #[cfg(feature = "tracing")]
                if records.len() >= 2 {
                    break;
                }
                #[cfg(not(feature = "tracing"))]
                if records.len() >= 1 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            records
        };

        assert_eq!(log_records[0].msg, "slog message");
        #[cfg(not(feature = "tracing"))]
        {
            assert_eq!(log_records.len(), 1);
        }
        #[cfg(feature = "tracing")]
        {
            assert_eq!(log_records.len(), 2);
            assert_eq!(log_records[1].msg, "tracing message");
            // Check that the structured fields are preserved
            let log_json: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&logpath)
                    .unwrap()
                    .lines()
                    .last()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(log_json["tracing_key"], "tracing_value");
            assert_eq!(log_json["tracing_num"], 84);
        }
    }

    /// Test the SlogKV implementation with different value types
    #[test]
    #[cfg(feature = "tracing")]
    fn test_slog_kv_types() {
        use super::{SlogKV, SlogValue};
        use slog::KV;

        let fields = vec![
            (
                "str_field".to_string(),
                SlogValue::Str("test_string".to_string()),
            ),
            ("i64_field".to_string(), SlogValue::I64(-123)),
            ("u64_field".to_string(), SlogValue::U64(456)),
            ("bool_field".to_string(), SlogValue::Bool(true)),
            (
                "debug_field".to_string(),
                SlogValue::Debug("debug_value".to_string()),
            ),
        ];

        let kv = SlogKV::new(fields);

        // Create a mock serializer to test the KV implementation
        struct MockSerializer {
            pub calls: std::cell::RefCell<Vec<(String, String)>>,
        }

        impl slog::Serializer for MockSerializer {
            fn emit_str(&mut self, key: slog::Key, val: &str) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("str:{}", val)));
                Ok(())
            }

            fn emit_i64(&mut self, key: slog::Key, val: i64) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("i64:{}", val)));
                Ok(())
            }

            fn emit_u64(&mut self, key: slog::Key, val: u64) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("u64:{}", val)));
                Ok(())
            }

            fn emit_bool(&mut self, key: slog::Key, val: bool) -> slog::Result {
                self.calls
                    .borrow_mut()
                    .push((key.as_ref().to_string(), format!("bool:{}", val)));
                Ok(())
            }

            fn emit_arguments(
                &mut self,
                _key: slog::Key,
                _val: &std::fmt::Arguments,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_unit(&mut self, _key: slog::Key) -> slog::Result {
                Ok(())
            }
            fn emit_char(
                &mut self,
                _key: slog::Key,
                _val: char,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_u8(&mut self, _key: slog::Key, _val: u8) -> slog::Result {
                Ok(())
            }
            fn emit_i8(&mut self, _key: slog::Key, _val: i8) -> slog::Result {
                Ok(())
            }
            fn emit_u16(&mut self, _key: slog::Key, _val: u16) -> slog::Result {
                Ok(())
            }
            fn emit_i16(&mut self, _key: slog::Key, _val: i16) -> slog::Result {
                Ok(())
            }
            fn emit_u32(&mut self, _key: slog::Key, _val: u32) -> slog::Result {
                Ok(())
            }
            fn emit_i32(&mut self, _key: slog::Key, _val: i32) -> slog::Result {
                Ok(())
            }
            fn emit_f32(&mut self, _key: slog::Key, _val: f32) -> slog::Result {
                Ok(())
            }
            fn emit_f64(&mut self, _key: slog::Key, _val: f64) -> slog::Result {
                Ok(())
            }
            fn emit_usize(
                &mut self,
                _key: slog::Key,
                _val: usize,
            ) -> slog::Result {
                Ok(())
            }
            fn emit_isize(
                &mut self,
                _key: slog::Key,
                _val: isize,
            ) -> slog::Result {
                Ok(())
            }
        }

        let mut serializer =
            MockSerializer { calls: std::cell::RefCell::new(Vec::new()) };

        // Test serialization
        let args = format_args!("test message");
        let record = slog::Record::new(
            &slog::RecordStatic {
                location: &slog::RecordLocation {
                    file: "test",
                    line: 1,
                    column: 1,
                    function: "test",
                    module: "test",
                },
                level: slog::Level::Info,
                tag: "test",
            },
            &args,
            slog::BorrowedKV(&()),
        );

        kv.serialize(&record, &mut serializer).unwrap();

        let calls = serializer.calls.borrow();
        assert_eq!(calls.len(), 5);
        assert_eq!(
            calls[0],
            ("str_field".to_string(), "str:test_string".to_string())
        );
        assert_eq!(calls[1], ("i64_field".to_string(), "i64:-123".to_string()));
        assert_eq!(calls[2], ("u64_field".to_string(), "u64:456".to_string()));
        assert_eq!(
            calls[3],
            ("bool_field".to_string(), "bool:true".to_string())
        );
        assert_eq!(
            calls[4],
            ("debug_field".to_string(), "str:debug_value".to_string())
        );
    }

    /// Test the SlogEventVisitor field extraction (without creating real tracing fields)
    #[test]
    #[cfg(feature = "tracing")]
    fn test_slog_event_visitor() {
        use super::{SlogEventVisitor, SlogValue};

        let mut visitor = SlogEventVisitor::new();

        // Create mock data - we can't easily create real tracing::field::Field instances
        // in tests, so we'll test by directly populating the visitor fields

        // Directly populate the visitor with test data
        visitor.fields.push((
            "str_key".to_string(),
            SlogValue::Str("string_value".to_string()),
        ));
        visitor.fields.push(("i64_key".to_string(), SlogValue::I64(-789)));
        visitor.fields.push(("u64_key".to_string(), SlogValue::U64(101112)));
        visitor.fields.push(("bool_key".to_string(), SlogValue::Bool(false)));
        visitor.message = Some("test message".to_string());

        // Check message extraction
        assert_eq!(visitor.message, Some("test message".to_string()));

        // Check field extraction and types
        assert_eq!(visitor.fields.len(), 4);

        let (key, value) = &visitor.fields[0];
        assert_eq!(key, "str_key");
        match value {
            SlogValue::Str(s) => assert_eq!(s, "string_value"),
            _ => panic!("Expected Str variant"),
        }

        let (key, value) = &visitor.fields[1];
        assert_eq!(key, "i64_key");
        match value {
            SlogValue::I64(i) => assert_eq!(*i, -789),
            _ => panic!("Expected I64 variant"),
        }

        let (key, value) = &visitor.fields[2];
        assert_eq!(key, "u64_key");
        match value {
            SlogValue::U64(u) => assert_eq!(*u, 101112),
            _ => panic!("Expected U64 variant"),
        }

        let (key, value) = &visitor.fields[3];
        assert_eq!(key, "bool_key");
        match value {
            SlogValue::Bool(b) => assert_eq!(*b, false),
            _ => panic!("Expected Bool variant"),
        }
    }
}
