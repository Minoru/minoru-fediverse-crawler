//! Macros to augment the logs.

/// Turns a string literal into a closure that returns the same literal, but prefixed with the
/// function's path and line number.
///
/// This is meant to be used in conjunction with `anyhow::Context::context`:
///
/// ```rust,no_run
/// foo.context(with_loc!("Unwrapping foo"))?;
/// ```
#[macro_export]
macro_rules! with_loc {
    ($msg: literal) => {
        format!("[{}:{}] {}", file!(), line!(), $msg)
    };
}
