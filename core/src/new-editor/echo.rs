/// The name of the Lisp variable that arms the echo area's timeout.
pub const ECHO_MESSAGE_TIMEOUT: &str = "echo-message-timeout";

/// How long an echo message stays on screen when nothing sets
/// [`ECHO_MESSAGE_TIMEOUT`] to something else.
pub const DEFAULT_ECHO_MESSAGE_TIMEOUT: f64 = 5.0;

/// What the echo area is showing, and since when.
///
/// The timestamp lives beside the text rather than in a lock of its own so
/// that a reader cannot catch a new message paired with the previous one's
/// clock and hide it a moment after it appeared.
#[derive(Debug, Clone, PartialEq)]
pub struct EchoMessage {
    pub text: String,
    /// When [`EditorState::set_echo_message`] last wrote `text`. Each new
    /// message restarts the clock, so a message is always shown for its full
    /// timeout however soon it followed the one before.
    pub set_at: Instant,
}

impl EchoMessage {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            set_at: Instant::now(),
        }
    }

    /// How long this message has been on screen, or `None` once TIMEOUT has
    /// run out -- and `None` too when there is no message to show, so that an
    /// empty echo area never counts as something waiting to expire.
    fn visible_for(&self, timeout: Option<Duration>) -> Option<Duration> {
        if self.text.is_empty() {
            return None;
        }
        let elapsed = self.set_at.elapsed();
        match timeout {
            Some(timeout) if elapsed >= timeout => None,
            _ => Some(elapsed),
        }
    }
}

/// The echo timeout as Lisp currently defines it, or `None` for "never
/// expires".
///
/// Only a finite, non-negative number arms the timeout. `nil` means the
/// message stays until something replaces it, and so does anything else --
/// an unbound variable, a string, a list, or an infinity. Refusing to guess
/// at a nonsensical value is the safe direction: the failure mode is a
/// message that outstays its welcome, not one that disappears before it is
/// read.
fn echo_timeout<B: BufferTrait>(env: &Arc<Env<EditorState<B>>>) -> Option<Duration> {
    match env.get_variable(ECHO_MESSAGE_TIMEOUT) {
        Some(ELispExp::Number(seconds)) if seconds.is_finite() => {
            Some(Duration::from_secs_f64(seconds.max(0.0)))
        }
        _ => None,
    }
}