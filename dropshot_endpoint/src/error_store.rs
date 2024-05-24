// Copyright 2024 Oxide Computer Company

//! Handle lists of errors in Dropshot.

use std::cell::RefCell;

#[derive(Debug)]
pub(crate) struct ErrorStore<T> {
    errors: Vec<T>,
    // A tree representing error sinks, so that we can track parent-child
    // relationships across sinks and propagate "has_errors" up the tree.
    sink_info: Vec<ErrorSinkState>,
}

impl<T> ErrorStore<T> {
    pub(crate) fn new() -> Self {
        Self { errors: Vec::new(), sink_info: Vec::new() }
    }

    pub(crate) fn into_inner(self) -> Vec<T> {
        self.errors
    }

    pub(crate) fn sink(&mut self) -> ErrorSink<'_, T> {
        self.sink_inner(None)
    }

    // --- Internal methods ---

    fn sink_inner(&mut self, parent: Option<usize>) -> ErrorSink<'_, T> {
        // len is the next ID
        let id = self.sink_info.len();
        self.sink_info.push(ErrorSinkState::new(parent));
        ErrorSink { errors: RefCell::new(self), id }
    }

    fn push(&mut self, id: usize, error: T) {
        self.errors.push(error);
        self.sink_info[id].has_errors = true;

        // Propagate the fact that errors were encountered up the tree.
        let mut curr = id;
        while let Some(parent) = self.sink_info[curr].parent {
            self.sink_info[parent].has_errors = true;
            curr = parent;
        }
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

#[derive(Debug)]
pub(crate) struct ErrorSink<'a, T> {
    // It's a bit weird to use both a lifetime parameter and a RefCell, but it
    // makes sense here. The lifetime parameter ensures that the error context
    // doesn't outlive the collector, and the RefCell exists so we can
    // arbitrarily nest contexts.
    //
    // (Without interior mutability, it becomes impossible to do so because when
    // it comes to mutable data, lifetime parameters can only express a static
    // number of levels of nesting. Note that this is not the case with
    // *immutable* data, where we can rely on covariance and so just use the
    // same `'a` everywhere.)
    errors: RefCell<&'a mut ErrorStore<T>>,
    id: usize,
}

impl<'a, T> ErrorSink<'a, T> {
    pub(crate) fn push(&self, error: T) {
        // This is always okay because we only briefly borrow the RefCell at any
        // time.
        self.errors.borrow_mut().push(self.id, error);
    }

    pub(crate) fn has_errors(&self) -> bool {
        // ErrorStore::push_error propagates has_errors up the tree while
        // writing errors, so we can just check the current ID while reading
        // this information.
        self.errors.borrow().sink_info[self.id].has_errors
    }

    pub(crate) fn new(&self) -> ErrorSink<'a, T> {
        let mut errors = self.errors.borrow_mut();
        let new = errors.sink_inner(Some(self.id));

        // SAFETY: The Rust compiler can't peer past the RefCell, but we know
        // ourselves that the new ErrorSink contains the same store, and so will
        // have the same lifetime, as the current one.
        unsafe {
            std::mem::transmute::<ErrorSink<'_, T>, ErrorSink<'a, T>>(new)
        }
    }
}
