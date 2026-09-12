;;; dired --- a directory in a buffer.
;;;
;;; A file manager written as a module: a mode, a keymap, and six commands. No
;;; part of the editor knows this exists. What makes it possible is a handful
;;; of primitives that are not about file managers at all -- `list-dir',
;;; `insert', `current-line', `set-buffer-read-only', `delete-file',
;;; `rename-file' -- and that is the interesting thing about it. A module can
;;; add a way of working with files without a line of Rust.
;;;
;;; # Where the state is
;;;
;;; Nowhere. There is no variable holding the current directory and no table
;;; mapping lines to file names. The buffer *is* the state: its first line says
;;; which directory this is a listing of, and each entry line carries a name.
;;; Every command reads back what is on screen.
;;;
;;; That is not a saving of two variables, it is the reason the module has no
;;; class of bug it would otherwise have. A directory remembered beside the
;;; buffer can disagree with the one displayed -- after a refresh that failed
;;; halfway, after the buffer is shown in a second window, after a command
;;; throws between updating one and the other -- and when they disagree, `D'
;;; deletes a file from a directory the user is not looking at. Reading the
;;; screen cannot be wrong about what the screen says.
;;;
;;; # One buffer
;;;
;;; Navigating replaces this buffer's contents rather than opening a second
;;; listing. Emacs keeps one buffer per directory; walking a tree there leaves
;;; a dozen behind. The cost of one buffer is that `C-x b' cannot take you back
;;; to a directory you left -- you retype it -- and that seems the better trade
;;; while the module has no history of its own.
;;;
;;; # The layout, and why it is so plain
;;;
;;;     /home/user/project/:
;;;       ..
;;;       Cargo.toml
;;;       core/
;;;       src/
;;;
;;; A header line ending in `:', then one entry per line indented by two
;;; spaces, directories marked with a trailing `/' by `list-dir'. Nothing else
;;; -- no size, no permissions, no date.
;;;
;;; The indent is what tells an entry line from the header, and the two are the
;;; only kinds of line there are, so `dired--name-here' can be four lines long
;;; and certain. Adding columns means the name is no longer the whole of the
;;; line, and every command that reads a name has to agree with every command
;;; that writes one about where it sits. That is a real cost to pay for
;;; information a file manager can also get by opening the file, so it waits
;;; until somebody wants it.

(make-mode 'dired-mode)

(defconst dired-buffer-name "*dired*"
  "The one buffer every listing is shown in. See the module header.")

;; ---------------------------------------------------------------------------
;; Paths
;; ---------------------------------------------------------------------------

(defun dired--as-directory (path)
  "PATH with exactly one trailing slash, so a name can be joined onto it.

Every path this module holds is in this form, which is what makes joining a
name onto it a `concat' and nothing more.

The empty path needs no case of its own: it does not end in a slash, so it gets
one, and \"/\" is the right answer for it."
  (if (string-suffix-p "/" path)
      path
      (concat path "/")))

(defun dired--parent (directory)
  "The directory above DIRECTORY. At the root, the root.

Written with `split-string' rather than by searching for the last slash
because this Lisp has no `string-match': splitting on \"/\" gives the
components, and dropping the last one is the answer. A trailing slash means the
last component is empty, so it is dropped first."
  (let ((parts (reverse (split-string (dired--as-directory directory) "/"))))
    ;; ("" "src" "project" "user" "home" "") reversed -- the empty head is the
    ;; trailing slash, and the one after it is the component to drop.
    (dired--rejoin (reverse (nthcdr 2 parts)))))

(defun dired--rejoin (parts)
  "PARTS, joined with slashes, as a directory path.

A slash goes *before* each part rather than between them, which needs no
special case for the first: the empty string that `split-string' peels off the
leading slash of an absolute path is skipped like any other empty part, and the
slash it stood for is put back by the next one.

Nothing is left over for the empty list either -- no parts means no slashes
means the empty path, and `dired--as-directory' is the one place that says what
an empty path means. A guard here returning \"/\" for it was a second answer to
the same question, agreeing with the first only by luck."
  (let ((path ""))
    (mapc (lambda (part)
            (unless (string= "" part)
              (setq path (concat path "/" part))))
          parts)
    (dired--as-directory path)))

;; ---------------------------------------------------------------------------
;; Reading the buffer back
;; ---------------------------------------------------------------------------

(defun dired--directory ()
  "The directory this listing is of, read out of its first line.

Point is put back where it was, so a command can ask this without moving the
cursor off the file the user is looking at."
  (let ((here (point)))
    (goto-line 1)
    (let ((header (current-line)))
      (goto-char here)
      ;; The header ends in the `:' that marks it as one.
      (dired--as-directory (substring header 0 -1)))))

(defun dired--name-here ()
  "The name on the line point is on, or nil if that line is not an entry.

An entry line is indented by two spaces and the header is not, which is the
whole of the distinction -- see the module header on why the layout stays that
plain."
  (let ((line (current-line)))
    (if (string-prefix-p "  " line)
        (substring line 2)
        nil)))

(defun dired--path-here ()
  "The full path of the entry point is on, or nil if point is not on one.

A directory keeps the trailing slash `list-dir' gave it, so that callers can
tell what they have without asking the filesystem a second time."
  (let ((name (dired--name-here)))
    (if name (concat (dired--directory) name) nil)))

;; ---------------------------------------------------------------------------
;; Showing a directory
;; ---------------------------------------------------------------------------

(defun dired--draw (directory)
  "Replace the dired buffer's contents with a listing of DIRECTORY.

The buffer is made writable for exactly as long as it takes to write, and
read-only again afterwards. That is the only window in which its text can
change at all -- the flag is enforced at the two functions every edit goes
through, so nothing else, command or keystroke or line of Lisp, gets in."
  (set-buffer-read-only nil)
  (clear-buffer)
  (insert directory ":\n")
  ;; `..' is listed, and not only reachable through `^', because a listing that
  ;; shows no way out of itself reads as a dead end. `list-dir' leaves it out
  ;; on purpose, so it is put back here.
  (insert "  ..\n")
  (mapc (lambda (entry) (insert "  " entry "\n")) (list-dir directory))
  (set-buffer-read-only t))

(defun dired--show (directory)
  "Show a listing of DIRECTORY in the dired buffer, and go to its first entry."
  (let ((directory (dired--as-directory (expand-file-name directory))))
    (buffer-create dired-buffer-name 'dired-mode)
    (switch-to-buffer dired-buffer-name)
    (dired--draw directory)
    ;; Line 2 is `..', the first thing that can be acted on. Line 1 is the
    ;; header, where every key that wants a file would report nothing.
    (goto-line 2)))

(defcommand dired (directory) ("fDired: ")
  "Show DIRECTORY in a buffer you can act on. Bound to C-x d.

RET opens what the cursor is on -- a file in this window, a directory as a new
listing. o opens a file in a window beside this one. ^ goes up. g re-reads. D
deletes and R renames, each asking first."
  (if (file-directory-p directory)
      (dired--show directory)
      (message "%s is not a directory" directory)))

(defcommand dired-refresh () nil
  "Re-read the directory being listed, keeping the cursor on the same line.

Which is the whole of how this module recovers from anything: every change to
the filesystem ends here, and the buffer is rebuilt from what is actually
there rather than being edited to match what was expected."
  (let ((here (line-number-at-point)))
    (dired--draw (dired--directory))
    (goto-line here)))

(defcommand dired-up-directory () nil
  "List the directory above the one being listed."
  (dired--show (dired--parent (dired--directory))))

;; ---------------------------------------------------------------------------
;; Opening
;; ---------------------------------------------------------------------------

(defun dired--visit (path)
  "Open PATH: a directory as a listing, a file in the current window.

The trailing slash `list-dir' puts on a directory is what decides, so this
does not need to ask the filesystem -- and `..', which has no slash, is sent
to `dired-up-directory' rather than being opened as the file it is not."
  (cond
   ((string= "  .." (current-line)) (dired-up-directory))
   ((string= "/" (substring path -1)) (dired--show path))
   (t (find-file path))))

(defcommand dired-find-file () nil
  "Open what the cursor is on, in this window. Bound to RET in dired-mode."
  (let ((path (dired--path-here)))
    (if path
        (dired--visit path)
        (message "No file on this line"))))

(defcommand dired-find-file-other-window () nil
  "Open what the cursor is on in a window beside this one, keeping the listing
visible. Bound to o in dired-mode.

Always a fresh split rather than reusing whatever other window happens to
exist: `o' pressed twice in a row should put the second file where the first
one went, and with a search for an existing window it would instead depend on
how the frame was divided, so the same keystroke would do different things on
different days."
  (let ((path (dired--path-here)))
    (cond
     ((null path) (message "No file on this line"))
     ;; A directory in another window would be a second listing in the one
     ;; buffer both windows show -- see the module header on why there is only
     ;; one. It opens here instead, which is what `RET' would have done.
     ((string= "/" (substring path -1)) (dired--visit path))
     (t (progn
          (split-window-right)
          (other-window 1)
          (find-file path))))))

;; ---------------------------------------------------------------------------
;; Changing the filesystem
;; ---------------------------------------------------------------------------
;;
;; Both of these ask before they act, and both ask with the minibuffer rather
;; than with a single keypress. A typed answer is the right weight for an
;; operation with no undo: the kill ring can give back a line, and nothing in
;; this editor can give back a file.
;;
;; The question is asked by opening a prompt whose confirm callback closes over
;; the path. Nothing is remembered in a variable between the question and the
;; answer -- which matters, because the user is free to move the cursor, or
;; refresh, or start another prompt, while the first one is open.

(defun dired--affirmative (answer)
  "Whether ANSWER is a yes. Anything else, including empty, is a no."
  (or (string= "yes" (downcase answer)) (string= "y" (downcase answer))))

(defun dired-delete-file ()
  "Delete what the cursor is on, after asking. Bound to D in dired-mode.

A directory with anything in it says how much would go with it, because
\"delete src/?\" is a question nobody can answer honestly."
  (let ((path (dired--path-here)))
    (cond
     ((null path) (message "No file on this line"))
     ((string= "  .." (current-line)) (message "Refusing to delete .."))
     (t (let* ((directory (string= "/" (substring path -1)))
               (count (if directory (directory-entry-count path) 0))
               ;; Whether this is the recursive question, decided now: the
               ;; answer the user gives is an answer to the question they were
               ;; shown, so what `delete-file' is allowed to do has to be
               ;; settled at the same moment the wording is.
               (deep (and directory count (> count 0))))
          (minibuffer-read
           (if deep
               (format "Recursively delete %s (%s entries)? (yes/no)" path count)
               (format "Delete %s? (yes/no)" path))
           (lambda (answer)
             (if (dired--affirmative answer)
                 (dired--deleted path (delete-file path deep))
                 (message "Not deleted")))
           nil nil))))))

(defun dired--deleted (path outcome)
  "Report the result of deleting PATH and re-read the listing.

The listing is re-read either way. A delete that failed leaves the file there,
and a listing still showing it is then correct -- but the refresh is what makes
that a statement rather than a coincidence, and it also picks up whatever else
changed on disk while the question was open."
  (if outcome
      (message "Deleted %s" path)
      (message "Could not delete %s -- see M-x switch-to-messages" path))
  (dired-refresh))

(defun dired-rename-file ()
  "Rename what the cursor is on, after reading the new name. Bound to R.

A name with no slash in it stays in this directory; one with a slash is a path,
which is how a file is moved somewhere else."
  (let ((path (dired--path-here))
        ;; Read now, and closed over, rather than read again when the answer
        ;; comes back. The question is about the directory that was listed when
        ;; it was asked -- and `dired--directory' answers about whatever buffer
        ;; is current, which during a callback is whatever the editor happens
        ;; to have restored.
        (directory (dired--directory))
        (name (dired--name-here)))
    (cond
     ((null path) (message "No file on this line"))
     ((string= "  .." (current-line)) (message "Refusing to rename .."))
     (t (minibuffer-read
         (format "Rename %s to:" name)
         (lambda (new-name)
           (dired--renamed
            path
            (rename-file path (dired--destination directory new-name))))
         nil nil)))))

(defun dired--destination (directory new-name)
  "Where a rename to NEW-NAME lands, for a file listed in DIRECTORY.

An absolute NEW-NAME, or one starting with `~\', is taken as given: that is how
a file is moved somewhere else entirely. Anything else is relative to
DIRECTORY -- so \"notes.md\" renames in place and \"archive/notes.md\" moves it
into a subdirectory of the listing.

Relative to the listing rather than to wherever the editor was started, which
is the only reading that matches what is on screen. The process working
directory is not something the user can see."
  (if (or (string-prefix-p "/" new-name) (string-prefix-p "~" new-name))
      (expand-file-name new-name)
      (concat directory new-name)))

(defun dired--renamed (path outcome)
  "Report the result of renaming PATH and re-read the listing."
  (if outcome
      (message "Renamed %s" path)
      (message "Could not rename %s -- see M-x switch-to-messages" path))
  (dired-refresh))

;; ---------------------------------------------------------------------------
;; Keys
;; ---------------------------------------------------------------------------
;;
;; Dired's own letters, which work because a mode's keymap is consulted before
;; the global one -- so `n' here is a movement and `n' elsewhere is still the
;; letter n. Everything this mode does not bind falls through, which is how
;; C-n, C-x o and M-x go on working inside a listing.
;;
;; Nothing needs to be unbound to protect the text. The buffer is read-only,
;; and that is enforced where edits happen rather than where keys are looked
;; up, so a printable key still reaches `self-insert' -- and `self-insert'
;; still says "Buffer is read-only" and changes nothing.

(define-key 'dired-mode "<ret>" 'dired-find-file)
(define-key 'dired-mode "o" 'dired-find-file-other-window)
(define-key 'dired-mode "^" 'dired-up-directory)
(define-key 'dired-mode "g" 'dired-refresh)
(define-key 'dired-mode "D" 'dired-delete-file)
(define-key 'dired-mode "R" 'dired-rename-file)

;; Movement without the control key, since a listing is read more than it is
;; edited. `q' leaves, which for a single-buffer listing means going back to
;; whatever was here before.
(define-key 'dired-mode "n" 'next-line)
(define-key 'dired-mode "p" 'previous-line)
(define-key 'dired-mode "q" '(close-buffer dired-buffer-name))

;; The way in. `C-x d' is dired's key in Emacs, and `C-x C-f' is already
;; find-file, so the two sit beside each other under the same prefix.
(define-key nil "C-x d" 'dired)

;; A listing is a view, so its faces are the ones a view uses: the header is
;; not text the user typed, and a directory is worth telling apart from a file
;; at a glance.
(add-syntax-rule 'dired-mode "^[^ ].*:$" 'keyword)
(add-syntax-rule 'dired-mode "^  (.*/)$" 'function 1)

(log "End of the dired.lisp")
