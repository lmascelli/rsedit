//! The stretch of a buffer's text that edits have reached since someone
//! started watching: how much at the top and how much at the bottom no edit
//! has touched, and what the stretch between held when watching began. Each
//! watcher has its own, under its key.
//!
//! The three doors tell every watcher of each edit, before the text moves. The text
//! kept is the stretch's, read as the stretch grows, not the buffer's: an
//! Insert session that types a word keeps the nothing that was there. Vim
//! asks it what a session changed, where it used to copy the whole buffer at
//! the start and compare the two copies at the end.
//!
//! Design: `doc/architettura-editor.typ, <due-porte>`.
use super::BufferTrait;

/// Who watches the edits to one buffer, each under its key: `None` is the
/// watcher that gives none. Empty when nobody watches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watchers(Vec<(Option<String>, Touched)>);

impl Watchers {
    /// Starts watching under KEY a text LENGTH characters long, forgetting
    /// what KEY watched before; the other watchers go on.
    pub fn begin(&mut self, key: Option<String>, length: usize) {
        let touched = Touched::new(length);
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, old)) => *old = touched,
            None => self.0.push((key, touched)),
        }
    }

    /// Stops watching under KEY, and returns what it watched, or `None`
    /// when KEY watched nothing.
    pub fn end(&mut self, key: Option<&str>) -> Option<Touched> {
        let at = self.0.iter().position(|(k, _)| k.as_deref() == key)?;
        Some(self.0.swap_remove(at).1)
    }

    /// Returns whether nobody watches.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Tells every watcher of an edit: see [`Touched::note`].
    pub fn note<B: BufferTrait>(&mut self, text: &B, from: usize, to: usize) {
        for (_, touched) in &mut self.0 {
            touched.note(text, from, to);
        }
    }

    /// Tells every watcher of a step of an undo or a redo: see
    /// [`Touched::note_unread`].
    pub fn note_unread(&mut self, from: usize, to: usize, length: usize) {
        for (_, touched) in &mut self.0 {
            touched.note_unread(from, to, length);
        }
    }

    /// Tells every watcher that every character was replaced: see
    /// [`Touched::note_all`].
    pub fn note_all(&mut self) {
        for (_, touched) in &mut self.0 {
            touched.note_all();
        }
    }
}

/// What the edits since watching began have reached, in one buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Touched {
    /// How long the text was when watching began.
    length: usize,
    /// The stretch edits have reached: `None` until the first edit.
    stretch: Option<Stretch>,
}

/// The stretch edits have reached.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stretch {
    /// How many characters at the start no edit has reached.
    head: usize,
    /// How many characters at the end no edit has reached.
    tail: usize,
    /// What the stretch held when watching began, or `None` once an undo or
    /// a redo has widened it: the history writes the text with the buffer
    /// out of reach, and what it went over cannot be read.
    before: Option<String>,
}

/// What changed while watching: from START, REMOVED characters of the text
/// as it was, REMOVED-TEXT, became INSERTED.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub removed: usize,
    pub inserted: String,
    /// The REMOVED characters, or `None` when they are no longer known.
    pub removed_text: Option<String>,
}

impl Touched {
    /// Starts watching a text LENGTH characters long.
    pub fn new(length: usize) -> Self {
        Self {
            length,
            stretch: None,
        }
    }

    /// Notes an edit about to replace FROM..TO of TEXT: a deletion of the
    /// range, or an insertion where FROM is TO.
    ///
    /// The text outside the stretch is as it was when watching began, so
    /// what the stretch takes in is read from TEXT, before it moves: only
    /// what lies between the stretch and the edit, never the whole text.
    pub fn note<B: BufferTrait>(&mut self, text: &B, from: usize, to: usize) {
        self.widen(from, to, text.len(), Some(text));
    }

    /// Notes a step of an undo or a redo about to replace FROM..TO of a text
    /// LENGTH characters long, which cannot be read: within the stretch it
    /// costs nothing, and past it what the stretch held is lost.
    pub fn note_unread(&mut self, from: usize, to: usize, length: usize) {
        self.widen::<crate::buffer::gap_buffer::GapBuffer>(from, to, length, None);
    }

    /// Notes that every character was replaced, as a file read again does.
    pub fn note_all(&mut self) {
        self.stretch = Some(Stretch {
            head: 0,
            tail: 0,
            before: None,
        });
    }

    /// Takes FROM..TO of a text LENGTH characters long into the stretch,
    /// reading what it adds from TEXT when there is one.
    ///
    /// The text after TO keeps its distance from the end through the edit,
    /// and the text before FROM its distance from the start, so each end
    /// shrinks to what this edit leaves alone.
    fn widen<B: BufferTrait>(&mut self, from: usize, to: usize, length: usize, text: Option<&B>) {
        let to = to.min(length);
        let from = from.min(to);
        let Some(stretch) = self.stretch.as_mut() else {
            self.stretch = Some(Stretch {
                head: from,
                tail: length - to,
                before: text.map(|text| text.slice(from, to)),
            });
            return;
        };
        let head = stretch.head.min(from);
        let tail = stretch.tail.min(length - to);
        if head < stretch.head || tail < stretch.tail {
            stretch.before = match (stretch.before.take(), text) {
                (Some(before), Some(text)) => {
                    let mut wider = text.slice(head, stretch.head);
                    wider.push_str(&before);
                    wider.push_str(&text.slice(length - stretch.tail, length - tail));
                    Some(wider)
                }
                _ => None,
            };
        }
        stretch.head = head;
        stretch.tail = tail;
    }

    /// Returns what changed, read from TEXT as it is now, or `None` when
    /// nothing has been edited.
    ///
    /// The same answer `string-diff` gives for the stretch: a text typed and
    /// deleted again leaves no change but where it was. After an undo or a
    /// redo that reached past the stretch, the whole stretch is the change,
    /// and the text it removed is not known.
    pub fn change<B: BufferTrait>(&self, text: &B) -> Option<Change> {
        let stretch = self.stretch.as_ref()?;
        let length = text.len();
        // Clamped, though the two ends cannot overlap in either text: each is
        // text no edit reached, and every edit lay between them.
        let start = stretch.head.min(self.length).min(length);
        let removed = self
            .length
            .saturating_sub(stretch.tail)
            .saturating_sub(start);
        let end = length.saturating_sub(stretch.tail).max(start);
        let after: Vec<char> = text.slice(start, end).chars().collect();
        let Some(before) = stretch.before.as_ref() else {
            return Some(Change {
                start,
                removed,
                inserted: after.into_iter().collect(),
                removed_text: None,
            });
        };
        let before: Vec<char> = before.chars().collect();
        let prefix = before
            .iter()
            .zip(&after)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = before[prefix..]
            .iter()
            .rev()
            .zip(after[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        Some(Change {
            start: start + prefix,
            removed: before.len() - prefix - suffix,
            inserted: after[prefix..after.len() - suffix].iter().collect(),
            removed_text: Some(before[prefix..before.len() - suffix].iter().collect()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::gap_buffer::GapBuffer;
    use crate::buffer::undo::{apply_delete, apply_insert};

    /// Notes and makes the insertion of CONTENT at AT.
    fn insert(touched: &mut Touched, text: &mut GapBuffer, at: usize, content: &str) {
        touched.note(text, at, at);
        apply_insert(text, at, content);
    }

    /// Notes and makes the deletion of FROM..TO.
    fn delete(touched: &mut Touched, text: &mut GapBuffer, from: usize, to: usize) {
        touched.note(text, from, to);
        apply_delete(text, from, to);
    }

    fn change(
        start: usize,
        removed: usize,
        inserted: &str,
        removed_text: Option<&str>,
    ) -> Option<Change> {
        Some(Change {
            start,
            removed,
            inserted: inserted.into(),
            removed_text: removed_text.map(Into::into),
        })
    }

    #[test]
    fn nothing_edited_is_no_change() {
        let text = GapBuffer::from("abcdefghij");
        assert_eq!(Touched::new(10).change(&text), None);
    }

    #[test]
    fn an_insertion_is_its_own_change() {
        let mut text = GapBuffer::from("abcdefghij");
        let mut touched = Touched::new(text.len());
        insert(&mut touched, &mut text, 4, "xyz");
        assert_eq!(touched.change(&text), change(4, 0, "xyz", Some("")));
    }

    #[test]
    fn edits_on_either_side_widen_the_change() {
        let mut text = GapBuffer::from("abcdefghij");
        let mut touched = Touched::new(text.len());
        insert(&mut touched, &mut text, 4, "x");
        insert(&mut touched, &mut text, 5, "y");
        // "abcdxyefghij": "cdx" goes, and "cd" with it from before.
        delete(&mut touched, &mut text, 2, 5);
        assert_eq!(text.to_string(), "abyefghij");
        assert_eq!(touched.change(&text), change(2, 2, "y", Some("cd")));
        // An edit far below takes in what lies between, as it was.
        insert(&mut touched, &mut text, 8, "!");
        assert_eq!(
            touched.change(&text),
            change(2, 7, "yefghi!", Some("cdefghi"))
        );
    }

    #[test]
    fn what_is_put_back_is_no_change() {
        // Replace state overwrites, and Backspace gives the characters back.
        let mut text = GapBuffer::from("abcdef");
        let mut touched = Touched::new(text.len());
        delete(&mut touched, &mut text, 2, 3);
        insert(&mut touched, &mut text, 2, "x");
        delete(&mut touched, &mut text, 2, 3);
        insert(&mut touched, &mut text, 2, "c");
        assert_eq!(touched.change(&text), change(3, 0, "", Some("")));
    }

    #[test]
    fn an_unread_step_past_the_stretch_makes_it_all_the_change() {
        let mut text = GapBuffer::from("abcdef");
        let mut touched = Touched::new(text.len());
        insert(&mut touched, &mut text, 3, "x");
        // Inside the stretch, the text before is still known.
        touched.note_unread(3, 4, text.len());
        apply_delete(&mut text, 3, 4);
        assert_eq!(touched.change(&text), change(3, 0, "", Some("")));
        // Past it, it is not.
        touched.note_unread(1, 2, text.len());
        apply_delete(&mut text, 1, 2);
        assert_eq!(touched.change(&text), change(1, 2, "c", None));
    }

    #[test]
    fn a_text_read_again_changed_everywhere() {
        let text = GapBuffer::from("xyz");
        let mut touched = Touched::new(10);
        touched.note_all();
        assert_eq!(touched.change(&text), change(0, 10, "xyz", None));
    }

    #[test]
    fn the_removed_text_is_what_the_stretch_held_between_what_stayed() {
        let mut text = GapBuffer::from("abcdef");
        let mut touched = Touched::new(text.len());
        delete(&mut touched, &mut text, 1, 4);
        insert(&mut touched, &mut text, 1, "bX");
        assert_eq!(touched.change(&text), change(2, 2, "X", Some("cd")));
    }

    #[test]
    fn each_key_watches_from_its_own_beginning() {
        let mut text = GapBuffer::from("abcdef");
        let mut watchers = Watchers::default();
        watchers.begin(None, text.len());
        watchers.note(&text, 0, 0);
        apply_insert(&mut text, 0, "x");
        watchers.begin(Some("log".into()), text.len());
        watchers.note(&text, 7, 7);
        apply_insert(&mut text, 7, "y");
        let log = watchers.end(Some("log")).unwrap();
        assert_eq!(log.change(&text), change(7, 0, "y", Some("")));
        // Ending one leaves the other watching, with every edit seen.
        assert!(!watchers.is_empty());
        let mine = watchers.end(None).unwrap();
        assert_eq!(mine.change(&text), change(0, 6, "xabcdefy", Some("abcdef")));
        assert!(watchers.is_empty());
    }

    #[test]
    fn beginning_again_under_a_key_forgets_only_what_it_watched() {
        let mut text = GapBuffer::from("abc");
        let mut watchers = Watchers::default();
        watchers.begin(None, text.len());
        watchers.begin(Some("log".into()), text.len());
        watchers.note(&text, 0, 0);
        apply_insert(&mut text, 0, "x");
        watchers.begin(None, text.len());
        assert_eq!(watchers.end(None).unwrap().change(&text), None);
        let log = watchers.end(Some("log")).unwrap();
        assert_eq!(log.change(&text), change(0, 0, "x", Some("")));
        assert_eq!(watchers.end(Some("log")), None);
    }
}
