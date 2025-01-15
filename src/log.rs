//! Log methods default to [indoc] format.
//!
//! # Examples  
//! ```
//! std_error!(
//!     "
//!     Write bot log to database failed: {e}
//!     Log: {content}
//!     "
//! );
//! ```
//!
//! which is equivalent to  
//! ```
//! kovi::log::error!("Write bot log to database failed: {e}\nLog: {content}")
//! ```
//!
//! Depending on demand, a log can be volatile or non-volatile.  
//! 1. kovi: use std_debug, std_info, std_warn, std_error  
//! 2. database: use db_debug, db_info, db_warn, db_error  
//! 3. both: use std_db_debug, std_db_info, std_db_warn, std_db_error  
//!
//! Pitfalls  
//! 1. db_* and std_db_* must be in async context  
//! 2. [indoc] does not trim trailing spaces

/// Append debug log entry to stdout
#[macro_export]
macro_rules! std_debug {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        kovi::log::debug!("{}", content);
    }};
}

/// Append info log entry to stdout
#[macro_export]
macro_rules! std_info {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        kovi::log::info!("{}", content);
    }};
}

/// Append warn log entry to stdout
#[macro_export]
macro_rules! std_warn {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        kovi::log::warn!("{}", content);
    }};
}

/// Append error log entry to stdout
#[macro_export]
macro_rules! std_error {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        kovi::log::error!("{}", content);
    }};
}

/// Append debug log entry to database.
#[macro_export]
macro_rules! db_debug {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        $crate::store::db_write_bot_log(time, "DEBUG".to_string(), content).await;
    }};
}

/// Append info log entry to database.
#[macro_export]
macro_rules! db_info {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        $crate::store::db_write_bot_log(time, "INFO".to_string(), content).await;
    }};
}

/// Append warn log entry to database.
#[macro_export]
macro_rules! db_warn {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        $crate::store::db_write_bot_log(time, "WARN".to_string(), content).await;
    }};
}

/// Append error log entry to database.
#[macro_export]
macro_rules! db_error {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        $crate::store::db_write_bot_log(time, "ERROR".to_string(), content).await;
    }};
}

/// Append debug log entry to stdout and database.
#[macro_export]
macro_rules! std_db_debug {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        kovi::log::debug!("{}", content);
        $crate::store::db_write_bot_log(time, "DEBUG".to_string(), content).await;
    }};
}

/// Append info log entry to stdout and database.
#[macro_export]
macro_rules! std_db_info {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        kovi::log::info!("{}", content);
        $crate::store::db_write_bot_log(time, "INFO".to_string(), content).await;
    }};
}

/// Append warn log entry to stdout and database.
#[macro_export]
macro_rules! std_db_warn {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        kovi::log::warn!("{}", content);
        $crate::store::db_write_bot_log(time, "WARN".to_string(), content).await;
    }};
}

/// Append error log entry to stdout and database.
#[macro_export]
macro_rules! std_db_error {
    ($($t:tt)*) => {{
        let content = indoc::formatdoc!($($t)*);
        let time = $crate::util::cur_time_iso8601();
        kovi::log::error!("{}", content);
        $crate::store::db_write_bot_log(time, "ERROR".to_string(), content).await;
    }};
}
