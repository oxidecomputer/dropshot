// Copyright 2025 Oxide Computer Company

use camino::Utf8PathBuf;

/// Context for validation of OpenAPI specifications.
pub struct ValidationContext<'a> {
    backend: &'a mut dyn ValidationBackend,
}

impl<'a> ValidationContext<'a> {
    /// Note part of the public API -- only called by the OpenAPI manager.
    #[doc(hidden)]
    pub fn new(backend: &'a mut dyn ValidationBackend) -> Self {
        Self { backend }
    }

    /// Reports a validation error.
    pub fn report_error(&mut self, error: anyhow::Error) {
        self.backend.report_error(error);
    }

    /// Records that the file has the given contents.
    ///
    /// In check mode, if the files differ, an error is logged.
    ///
    /// In generate mode, the file is overwritten with the given contents.
    ///
    /// The path is treated as relative to the root of the repository.
    pub fn record_file_contents(
        &mut self,
        path: impl Into<Utf8PathBuf>,
        contents: Vec<u8>,
    ) {
        self.backend.record_file_contents(path.into(), contents);
    }
}

/// The backend for validation.
///
/// Not part of the public API -- only implemented by the OpenAPI manager.
#[doc(hidden)]
pub trait ValidationBackend {
    fn report_error(&mut self, error: anyhow::Error);
    fn record_file_contents(&mut self, path: Utf8PathBuf, contents: Vec<u8>);
}
