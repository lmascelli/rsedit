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

(defun dired--parent (directory)
  "The directory above DIRECTORY. At the root, the root.

Asked rather than computed. This module used to work it out by splitting on
\"/\" and dropping the last component, which is one platform's rule written down
as though it were the rule -- on Windows a path has no \"/\" in it to split on,
so every answer was the root. It also had no way to be right about a *relative*
path: \"./\" has one component, dropping it leaves nothing, and nothing renders
as the root. That is the whole of the bug where `^' jumped to \"/\".

The two primitives are what make the first half impossible: they ask the
platform what a separator is instead of assuming. `expand-file-name' is what
makes the second half impossible -- and it is applied here rather than relied
on from the caller, so that this function has an answer for every input rather
than only for the ones it currently gets. A relative DIRECTORY resolves against
the editor's working directory, which is the same reading `dired' itself gives
one."
  (file-name-directory (directory-file-name (expand-file-name directory))))

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
      (file-name-as-directory (substring header 0 -1)))))

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
  ;; Expanded first: what arrives here may be relative -- typed as ".", or
  ;; completed from the prompt -- and a relative path in the header is a path
  ;; whose parent cannot be worked out.
  (let ((directory (file-name-as-directory (expand-file-name directory))))
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

`file-directory-p' decides, rather than the trailing separator `list-dir' puts
on a directory. The separator is there to be *read* -- it is what tells you at a
glance that Enter will descend -- and reading it back to make the decision meant
writing down which character it is, which is the platform's business and not
this module's. Asking costs one call and is also simply the truth: the listing
may be a few seconds old.

`..' is sent up rather than opened as the directory it also is, so that the
header ends up normalised instead of growing a `..' on the end."
  (cond
   ((string= "  .." (current-line)) (dired-up-directory))
   ((file-directory-p path) (dired--show path))
   (t (find-file path))))

(defcommand dired-find-file () nil
  "Open what the cursor is on, in this window. Bound to RET in dired-mode."
  (let ((path (dired--path-here)))
    (if path
        (dired--visit path)
        (message "No file on this line"))))

(defconst dired-window-width 40
  "How many columns the listing keeps when `o' opens a file beside it.

A column count rather than a fraction of the frame. A listing is as wide as the
file names in it, which does not change when the terminal is resized -- so 30%
of a wide terminal is more than the listing needs and 30% of a narrow one is
too little to read. Fixing the listing also means the *file* absorbs a resize,
which is the right way round: that is the window being worked in.")

;; The window `o' last opened a file into, so that pressing it again replaces
;; that file instead of splitting the frame a second time. nil when `o' has not
;; been pressed, or when the window it used has since been closed.
(setq dired--file-window nil)

(defcommand dired-find-file-other-window () nil
  "Open what the cursor is on in a window beside this one, keeping the listing
visible. Bound to o in dired-mode.

The first `o' splits, leaving the listing `dired-window-width' columns wide.
Every `o' after that reuses the window the last one opened, so walking down a
directory reading files leaves two windows rather than a frame sliced into
eight. If that window has since been closed, the next `o' splits again."
  (let ((path (dired--path-here)))
    (cond
     ((null path) (message "No file on this line"))
     ;; A directory in another window would be a second listing in the one
     ;; buffer both windows show -- see the module header on why there is only
     ;; one. It opens here instead, which is what `RET' would have done.
     ((file-directory-p path) (dired--visit path))
     (t (dired--visit-other-window path)))))

(defun dired--visit-other-window (path)
  "Open PATH beside the listing, reusing the window `o' last used if it is
still there."
  ;; `select-window' answers nil for a window that has been closed since, which
  ;; is what makes "reuse it if it is still there" a single question rather than
  ;; a search through the layout.
  (if (and dired--file-window (select-window dired--file-window))
      (find-file path)
      (progn
        (split-window-right dired-window-width)
        (other-window 1)
        (setq dired--file-window (selected-window))
        (find-file path))))

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
     (t (let* ((directory (file-directory-p path))
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

One call, because this is exactly what `expand-file-name' second argument is
for: an absolute NEW-NAME is taken as given, a `~' is expanded, and anything
else is resolved against DIRECTORY. So \"notes.md\" renames in place,
\"archive/notes.md\" moves it into a subdirectory of the listing, and
\"/tmp/notes.md\" moves it out entirely.

Against the listing rather than against wherever the editor was started, which
is the only reading that matches what is on screen -- the process working
directory is not something the user can see.

This used to test for a leading \"/\" to decide whether NEW-NAME was absolute,
which is a rule that holds on exactly one family of platforms."
  (expand-file-name new-name directory))

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


;; ---------------------------------------------------------------------------
;; Finding a file anywhere below here
;; ---------------------------------------------------------------------------
;;
;; Folded into this module rather than standing as one of its own. It is a way
;; of getting at files, which is what this module is for, and a module per
;; feature is how a configuration becomes a directory of one-function files
;; nobody can find anything in.
;;
;; The walk itself is `directory-files-recursive', a primitive, because it has
;; to be: pruning cannot be done afterwards. `.git' in a working repository
;; holds thousands of objects, and a walk that collected them would hit its
;; limit before reaching a single source file -- the files you wanted would not
;; be in the list to filter.

(defconst find-file-recursive-ignore
  '(".git" ".hg" ".svn" ".jj"
    "target" "build" "dist" "out"
    "node_modules" "vendor" ".venv" "venv" "__pycache__"
    ".cache" ".mypy_cache" ".pytest_cache" ".tox")
  "Directory names the search never descends into.

Names, not paths: `target' is skipped wherever it appears, at any depth. These
are the directories that are large *and* generated -- what is in them is either
a copy of something else or rebuildable, so finding a file there is almost
never what was meant.")

(defconst find-file-recursive-limit 5000
  "How many files the search collects before it stops looking.

Not a performance guard so much as a usability one: a completion strip with
fifty thousand entries in it is not a way to find anything, and the search that
built it would have spent a second doing so. When the limit is reached the
search says so rather than presenting a partial list as though it were
everything.")

;; ---------------------------------------------------------------------------
;; .gitignore, approximately
;; ---------------------------------------------------------------------------
;;
;; A first approximation on purpose, and these are the parts it does not do:
;; negation (`!keep-this'), `**' path patterns, patterns anchored with a
;; leading `/', and per-directory .gitignore files below the root. What it does
;; handle is what the overwhelming majority of lines in a real .gitignore
;; actually are: a bare name, a name with a trailing slash, and `*.extension'.
;;
;; Doing it properly means implementing gitignore's matching rules, which are
;; genuinely intricate. Doing it approximately means the search occasionally
;; offers a file git would have hidden -- which is a much smaller problem than
;; the search hiding a file you were looking for, and is why nothing here ever
;; *removes* a candidate on a pattern it only half understood.

(defun find-file-recursive--gitignore-lines (directory)
  "The meaningful lines of DIRECTORY's .gitignore, or nil if it has none."
  (let ((text (read-file-to-string (concat (file-name-as-directory directory)
                                           ".gitignore"))))
    (if (null text)
        nil
        ;; Blank lines and comments dropped here so that nothing downstream has
        ;; to keep checking for them.
        ;; Built with `cons' and reversed once at the end rather than with
        ;; `append', which walks everything gathered so far on each element and
        ;; so costs the square of the list's length. See
        ;; `find-file-recursive--candidates' for where that mattered.
        (let ((kept nil))
          (mapc (lambda (line)
                  (let ((trimmed (find-file-recursive--trim line)))
                    (if (and (not (string= trimmed ""))
                             (not (string= (substring trimmed 0 1) "#")))
                        (setq kept (cons trimmed kept)))))
                (split-string text "\n"))
          (reverse kept)))))

(defun find-file-recursive--trim (text)
  "TEXT without leading or trailing spaces, tabs or carriage returns.

The carriage return matters: a .gitignore written on Windows ends every line
with one, and `target\\r' matches no directory at all."
  (let ((start 0)
        (end (length text)))
    (while (and (< start end)
                (member (substring text start (+ start 1)) '(" " "\t" "\r")))
      (setq start (+ start 1)))
    (while (and (< start end)
                (member (substring text (- end 1) end) '(" " "\t" "\r")))
      (setq end (- end 1)))
    (substring text start end)))

(defun find-file-recursive--ignored-directories (directory)
  "Directory names DIRECTORY's .gitignore asks to be left out.

A pattern counts as a directory when it ends in `/', or when it is a plain name
with no glob character in it -- git would treat the second as matching a file
*or* a directory, and treating it as both here costs nothing: a file of that
name is dropped by the suffix rules below anyway."
  (let ((names nil))
    (mapc (lambda (pattern)
            ;; Anchored and multi-segment patterns are the ones this does not
            ;; understand. Skipped rather than guessed at.
            (if (and (not (member "!" (list (substring pattern 0 1))))
                     (null (member "/" (find-file-recursive--characters
                                        (find-file-recursive--strip-trailing-slash pattern))))
                     (null (member "*" (find-file-recursive--characters pattern))))
                (setq names
                      (append names
                              (list (find-file-recursive--strip-trailing-slash pattern))))))
          (find-file-recursive--gitignore-lines directory))
    names))

(defun find-file-recursive--characters (text)
  "TEXT as a list of one-character strings."
  (let ((out nil)
        (n 0))
    (while (< n (length text))
      (setq out (cons (substring text n (+ n 1)) out))
      (setq n (+ n 1)))
    (reverse out)))

(defun find-file-recursive--strip-trailing-slash (pattern)
  "PATTERN without its trailing `/', if it has one."
  (if (and (> (length pattern) 0)
           (string= (substring pattern (- (length pattern) 1)) "/"))
      (substring pattern 0 (- (length pattern) 1))
      pattern))

(defun find-file-recursive--ignored-suffixes (directory)
  "The extensions DIRECTORY's .gitignore asks to be left out.

Only `*.something' is recognised, and only as a suffix. A pattern with a `*'
anywhere else is skipped -- half-understanding a glob and dropping files on the
strength of it is how a search comes to not find a file that is there."
  (let ((suffixes nil))
    (mapc (lambda (pattern)
            (if (and (> (length pattern) 2)
                     (string= (substring pattern 0 2) "*.")
                     (null (member "*" (find-file-recursive--characters
                                        (substring pattern 1))))
                     (null (member "/" (find-file-recursive--characters pattern))))
                (setq suffixes (cons (substring pattern 1) suffixes))))
          (find-file-recursive--gitignore-lines directory))
    (reverse suffixes)))

;; ---------------------------------------------------------------------------
;; The command
;; ---------------------------------------------------------------------------

(defun find-file-recursive--candidates (directory)
  "Every file under DIRECTORY worth offering, as (TRUNCATED PATHS).

Both kinds of exclusion are handed to the walk rather than applied to what it
returns. That is not tidiness: filtering here costs one evaluation per file per
pattern, and a 5,000-file tree whose .gitignore names forty `*.ext' patterns
spent more than a whole command's fuel budget doing it -- so the command
returned nothing at all, which looked like a project with no files in it."
  (directory-files-recursive directory
                             find-file-recursive-limit
                             (append find-file-recursive-ignore
                                     (find-file-recursive--ignored-directories directory))
                             (find-file-recursive--ignored-suffixes directory)))

(defun dired-find-recursive--candidates (input)
  "File names under the working directory matching INPUT, best first.

The tree is walked once, when the prompt opens, and narrowed from that snapshot
as you type. Walking on every keystroke would be thousands of directory reads
per character on a real project, and the prompt would stutter exactly where it
is most wanted. The cost is that a file created while the prompt is open does
not appear, which is almost never what is being looked for."
  (fuzzy-filter input dired--recursive-candidates))

(defcommand dired-find-recursive () nil
  "Open a file from anywhere under the working directory, choosing it by name.

Bound to C-x C-r. Where `C-x C-f' walks the tree a directory at a time, this
flattens it: type any part of the name and the completion narrows to it.

`.git', `target', `node_modules' and the rest of `find-file-recursive-ignore'
are never descended into, and the working directory's .gitignore is read for
more. If the search stops at `find-file-recursive-limit' it says so -- a
truncated list looks exactly like a complete one, and a file missing from it
looks exactly like a file that is not there."
  (let* ((found (find-file-recursive--candidates "."))
         (truncated (car found))
         (paths (nth 1 found)))
    (cond
     ((null paths) (message "No files under %s" (expand-file-name ".")))
     (t (progn
          ;; Held for the life of the prompt. The completion function reads it
          ;; rather than walking again, which is what keeps typing responsive.
          (setq dired--recursive-candidates paths)
          (if truncated
              (message "%s files (truncated at %s -- narrow the directory or raise find-file-recursive-limit)"
                       (length paths) find-file-recursive-limit))
          (minibuffer-read "Find file recursively:"
                           (lambda (path)
                             (setq dired--recursive-candidates nil)
                             (if (string= path "")
                                 nil
                                 (find-file path)))
                           'dired-find-recursive--candidates
                           nil))))))

;; The snapshot the prompt narrows. Emptied when the prompt is answered, so a
;; stale list cannot be offered to a later one.
(setq dired--recursive-candidates nil)

;; The way in. `C-x d' is dired's key in Emacs, and `C-x C-f' is already
;; find-file, so the two sit beside each other under the same prefix.
(define-key nil "C-x d" 'dired)
(define-key nil "C-x C-r" 'dired-find-recursive)

;; What `find-file' hands a directory to. The editor knows only that this
;; variable exists; that dired is what answers it is this line and nothing
;; else, so a different file manager can take the job by setting it too.
(setq *open-directory-callback* 'dired)

;; A listing is a view, so its faces are the ones a view uses: the header is
;; not text the user typed, and a directory is worth telling apart from a file
;; at a glance.
(add-syntax-rule 'dired-mode "^[^ ].*:$" 'keyword)
(add-syntax-rule 'dired-mode "^  (.*/)$" 'function 1)

(log "End of the dired.lisp")
