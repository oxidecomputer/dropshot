// Copyright 2024 Oxide Computer Company

//! Handle lists of errors that occur while generating the proc macro.
//!
//! See the documentation of [`ErrorStore`] for more information.

use std::cell::RefCell;

/// Top-level struct that holds all errors encountered during the invocation of
/// a proc macro.
///
/// ## Motivation
///
/// Dropshot's proc macros have several components that can all independently
/// produce errors. We generally would like to make as much progress as possible
/// within each of the components, because this tends to lead to better errors.
///
/// For example, the `endpoint` macro parses two components: the metadata, and
/// the function signature.
///
/// * Failing to parse the metadata should not prevent us from parsing the
///   function signature, and vice versa.
/// * Within the signature, each type should be considered separately -- so an
///   issue in an extractor type shouldn't stop us from parsing the return type.
/// * But within a section, if there are errors, we should avoid producing the
///   corresponding output -- because doing so can lead to confusing errors.
///
/// So the typical Rust pattern of returning on the first error isn't quite good
/// enough for us, and what we need is:
///
/// * a hierarchical way to collect errors
/// * with arbitrary nesting -- in other words, a tree of error collectors
/// * and each node in the tree tracks whether any errors are attributable to
///   it, or its descendants
///
/// This is what `ErrorStore` provides.
///
/// * An `ErrorStore` represents a top-level store of errors, and each store can
/// have one or more [`ErrorSink`] instances.
/// * Each `ErrorSink` represents a context in which errors can be collected,
///   and can have its own child `ErrorSink` instances.
/// * Errors are pushed to `ErrorSink` instances, and the `ErrorStore` tracks
///   whether any errors were pushed to a given `ErrorSink` or its descendants.
#[derive(Debug)]
pub(crate) struct ErrorStore<T> {
    data: RefCell<ErrorStoreData<T>>,
}

impl<T> ErrorStore<T> {
    pub(crate) fn new() -> Self {
        Self { data: RefCell::new(ErrorStoreData::default()) }
    }

    pub(crate) fn into_inner(self) -> Vec<T> {
        std::mem::take(&mut self.data.borrow_mut().errors)
    }

    pub(crate) fn sink(&mut self) -> ErrorSink<'_, T> {
        let new_id = self.data.borrow_mut().register_sink(None);
        ErrorSink { data: &self.data, id: new_id }
    }
}

#[derive(Debug)]
struct ErrorStoreData<T> {
    errors: Vec<T>,
    sinks: Vec<ErrorSinkState>,
}

impl<T> Default for ErrorStoreData<T> {
    fn default() -> Self {
        Self { errors: Vec::new(), sinks: Vec::new() }
    }
}

impl<T> ErrorStoreData<T> {
    fn push(&mut self, id: usize, error: T) {
        self.errors.push(error);
        self.sinks[id].has_errors = true;

        // Propagate the fact that errors were encountered up the tree.
        let mut curr = id;
        while let Some(parent) = self.sinks[curr].parent {
            self.sinks[parent].has_errors = true;
            curr = parent;
        }
    }

    // --- Internal methods ---

    fn register_sink(&mut self, parent: Option<usize>) -> usize {
        // len is the next ID
        let id = self.sinks.len();
        self.sinks.push(ErrorSinkState::new(parent));
        id
    }
}

#[derive(Debug)]
struct ErrorSinkState {
    // The parent ID in the map.
    parent: Option<usize>,
    // Whether an error was pushed via this specific context or a descendant.
    has_errors: bool,
}

impl ErrorSinkState {
    fn new(parent: Option<usize>) -> Self {
        Self { parent, has_errors: false }
    }
}

/// A collector for errors.
///
/// An `ErrorSink` is a context into which errors can be collected. It can have
/// child `ErrorSink` instances, and the [`ErrorStore`] from which it is
/// ultimately derived tracks whether any errors were pushed to a given
/// `ErrorSink` or its descendants.
///
/// The lifetime parameter `'a` is the lifetime of the `ErrorStore` that the
/// `ErrorSink` is ultimately derived from. The parameter ensures that
/// `ErrorSink` instances don't outlive the [`ErrorStore`] -- this means that at
/// the time an [`ErrorStore`] is consumed, there aren't any outstanding
/// `ErrorSink` instances.
#[derive(Debug)]
pub(crate) struct ErrorSink<'a, T> {
    // It's a bit weird to use both a lifetime parameter and a RefCell, but it
    // makes sense here. With `Rc<RefCell<T>>`, there's no way to statically
    // guarantee that the error collection process is done. The lifetime
    // parameter statically guarantees that.
    //
    // Do we need interior mutability? Because of our nested structure, we'd
    // have to either use `&mut &mut &mut ... T`, or dynamic dispatch. Both seem
    // worse than just doing this.
    data: &'a RefCell<ErrorStoreData<T>>,
    id: usize,
}

impl<'a, T> ErrorSink<'a, T> {
    pub(crate) fn push(&self, error: T) {
        // This is always okay because we only briefly borrow the RefCell at any
        // time.
        self.data.borrow_mut().push(self.id, error);
    }

    pub(crate) fn has_errors(&self) -> bool {
        // ErrorStore::push_error propagates has_errors up the tree while
        // writing errors, so we can just check the current ID while reading
        // this information.
        self.data.borrow().sinks[self.id].has_errors
    }

    pub(crate) fn new(&self) -> ErrorSink<'a, T> {
        let mut errors = self.data.borrow_mut();
        let new_id = errors.register_sink(Some(self.id));
        Self { data: self.data, id: new_id }
    }
}
