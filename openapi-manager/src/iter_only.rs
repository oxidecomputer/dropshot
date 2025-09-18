// Copyright 2025 Oxide Computer Company

//! Extract the only item from an iterator

use std::fmt::Debug;
use thiserror::Error;

/// Error returned by [`iter_only`]
#[derive(Debug, Error)]
pub enum OnlyError {
    #[error("list was unexpectedly empty")]
    Empty,

    #[error(
        "found at least two elements in a list that was expected to contain \
         exactly one: {0} {1}"
    )]
    // Store the debug representations directly here rather than the values
    // so that `OnlyError: 'static` (so that it can be used as the cause of
    // another error) even when `T` is not 'static.
    Extra(String, String),
}

/// Extract the only item from an iterator, failing if there are 0 or more than
/// one item
pub fn iter_only<T: Debug>(
    mut iter: impl Iterator<Item = T>,
) -> Result<T, OnlyError> {
    let first = iter.next().ok_or(OnlyError::Empty)?;
    match iter.next() {
        None => Ok(first),
        Some(second) => Err(OnlyError::Extra(
            format!("{:?}", first),
            format!("{:?}", second),
        )),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_basic() {
        assert_matches!(
            iter_only::<u8>(std::iter::empty()),
            Err(OnlyError::Empty)
        );
        assert_matches!(
            iter_only([8u8, 12, 15].iter()),
            Err(OnlyError::Extra(one, two)) if one == "8" && two == "12"
        );
        assert_eq!(*iter_only([8].iter()).unwrap(), 8);
    }
}
