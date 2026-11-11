//! File access causes and recovery history, stored as buffer attachments.
//! Rendering and status commands use these snapshots without filesystem I/O.
//! Design: doc/architettura-editor.typ, <file-su-disco>.
use super::{
    Buffer, BufferTrait,
    disk::{IO_ERROR, IO_MARK, IoMark},
};
use std::{
    fmt,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const AUTO_SAVE_REPORT: &str = "file-io:auto-save-report";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IoCause {
    Missing,
    Denied,
    Network,
    Timeout,
    ReadOnly,
    Full,
    Busy,
    InvalidPath,
    Configuration,
    Uncertain,
    Other,
}
impl IoCause {
    pub(crate) fn short(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Denied => "denied",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::ReadOnly => "read-only",
            Self::Full => "full",
            Self::Busy => "busy",
            Self::InvalidPath => "invalid path",
            Self::Configuration => "config",
            Self::Uncertain => "uncertain",
            Self::Other => "error",
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Missing => "File missing",
            Self::Denied => "Access denied",
            Self::Network => "Network unavailable",
            Self::Timeout => "I/O timed out",
            Self::ReadOnly => "Read-only filesystem",
            Self::Full => "No space left",
            Self::Busy => "I/O busy",
            Self::InvalidPath => "Invalid path",
            Self::Configuration => "Recovery configuration error",
            Self::Uncertain => "Outcome uncertain",
            Self::Other => "I/O error",
        }
    }
    fn of(error: &Error) -> Self {
        // ESTALE has no portable ErrorKind. Windows can map unavailable SMB
        // shares to NotFound: classify their native codes before that kind.
        if (cfg!(target_os = "linux") && error.raw_os_error() == Some(116))
            || (cfg!(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
                target_os = "dragonfly"
            )) && error.raw_os_error() == Some(70))
            || (cfg!(windows)
                && matches!(
                    error.raw_os_error(),
                    Some(53 | 64 | 67 | 1231 | 1232 | 1233 | 10050 | 10051 | 10053 | 10054 | 10057)
                ))
        {
            return Self::Network;
        }
        if cfg!(windows) && matches!(error.raw_os_error(), Some(121 | 10060)) {
            return Self::Timeout;
        }
        match error.kind() {
            ErrorKind::NotFound => Self::Missing,
            ErrorKind::PermissionDenied => Self::Denied,
            ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::NetworkUnreachable
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkDown
            | ErrorKind::BrokenPipe => Self::Network,
            ErrorKind::TimedOut => Self::Timeout,
            ErrorKind::ReadOnlyFilesystem => Self::ReadOnly,
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => Self::Full,
            ErrorKind::WouldBlock | ErrorKind::ResourceBusy => Self::Busy,
            ErrorKind::NotADirectory | ErrorKind::IsADirectory | ErrorKind::InvalidInput => {
                Self::InvalidPath
            }
            _ => Self::Other,
        }
    }
}

/// Preserve the OS information even after its Error has crossed a worker boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileIssue {
    pub cause: IoCause,
    pub kind: Option<ErrorKind>,
    pub os_code: Option<i32>,
    pub message: String,
}
impl FileIssue {
    pub(crate) fn from_io(error: &Error) -> Self {
        Self {
            cause: IoCause::of(error),
            kind: Some(error.kind()),
            os_code: error.raw_os_error(),
            message: error.to_string(),
        }
    }
    pub(crate) fn new(cause: IoCause, message: impl Into<String>) -> Self {
        Self {
            cause,
            kind: None,
            os_code: None,
            message: message.into(),
        }
    }
    pub(crate) fn detail(&self) -> String {
        format!(
            "{}: {} (kind: {:?}, OS code: {:?})",
            self.cause.label(),
            self.message,
            self.kind,
            self.os_code
        )
    }
}
impl fmt::Display for FileIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) fn is_missing(error: &Error) -> bool {
    IoCause::of(error) == IoCause::Missing
}

#[derive(Clone, Debug)]
pub(crate) struct AutoSaveReport {
    pub at: SystemTime,
    pub version: u64,
    pub copy: Option<PathBuf>,
    pub outcome: Result<(), FileIssue>,
    pub last_success: Option<SystemTime>,
    pub removed: bool,
}

// Floor negative fractional seconds too, for dates before the Unix epoch.
fn seconds(at: SystemTime) -> i128 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(span) => i128::from(span.as_secs()),
        Err(error) => {
            -i128::from(error.duration().as_secs())
                - i128::from(error.duration().subsec_nanos() != 0)
        }
    }
}
fn clock(at: SystemTime) -> String {
    let s = seconds(at).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}
pub(crate) fn utc(at: SystemTime) -> String {
    // Gregorian civil date from days since 1970; no platform timezone calls.
    let z = seconds(at).div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let day = z - era * 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let doy = day - (365 * year + year / 4 - year / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = year + era * 400 + i128::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {} UTC",
        clock(at).trim_end_matches('Z')
    )
}

impl<B: BufferTrait> Buffer<B> {
    pub(crate) fn note_auto_save(
        &mut self,
        at: SystemTime,
        version: u64,
        copy: Option<&Path>,
        outcome: Result<(), FileIssue>,
    ) {
        let last_success = if outcome.is_ok() {
            Some(at)
        } else {
            self.data::<AutoSaveReport>(AUTO_SAVE_REPORT)
                .and_then(|report| report.last_success)
        };
        self.auto_save_failing = outcome.is_err();
        self.put_data(
            AUTO_SAVE_REPORT,
            AutoSaveReport {
                at,
                version,
                copy: copy.map(Path::to_path_buf),
                outcome,
                last_success,
                removed: false,
            },
        );
    }
    pub(crate) fn auto_save_status(&self) -> String {
        let Some(report) = self.data::<AutoSaveReport>(AUTO_SAVE_REPORT) else {
            return "Auto-save: never attempted in this buffer".into();
        };
        let result = match &report.outcome {
            Ok(()) => "OK".into(),
            Err(why) => format!("failed: {}", why.detail()),
        };
        let mut copy = report
            .copy
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "not configured".into());
        if report.removed {
            copy.push_str(" (removed after successful save)");
        }
        let success = report
            .last_success
            .map(utc)
            .unwrap_or_else(|| "never".into());
        format!(
            "Auto-save: {result}; attempt: {}; version: {}; copy: {copy}; last success: {success}",
            utc(report.at),
            report.version
        )
    }
    pub(crate) fn file_io_status(&self) -> String {
        let mut status = String::new();
        if let Some(marks) = self.data::<Vec<IoMark>>(IO_MARK) {
            for mark in marks {
                status.push_str(&format!(
                    " [{}{}]",
                    mark.label,
                    if mark.slow { "?" } else { "" }
                ));
            }
        }
        if let Some(issue) = self.data::<FileIssue>(IO_ERROR) {
            status.push_str(&format!(" [{}]", issue.cause.label()));
        }
        if self.stale {
            status.push_str(" [File conflict]");
        }
        if let Some(report) = self.data::<AutoSaveReport>(AUTO_SAVE_REPORT) {
            let result = report
                .outcome
                .as_ref()
                .err()
                .map(|why| why.cause.short())
                .unwrap_or("ok");
            status.push_str(&format!(" [AS {result} {}]", clock(report.at)));
        } else if self.auto_save_failing {
            status.push_str(" [Auto-save failed]");
        } else if self.file_path.is_some() {
            status.push_str(" [AS never]");
        }
        status
    }
    pub(crate) fn file_io_detail(&self) -> String {
        let file = self
            .data::<FileIssue>(IO_ERROR)
            .map(FileIssue::detail)
            .unwrap_or_else(|| "no recorded I/O error".into());
        format!(
            "File: {file};{}; {}",
            self.file_io_status(),
            self.auto_save_status()
        )
    }
}
