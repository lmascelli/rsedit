;;; occur --- listing every line that matches, and walking the list.
;;;
;;; `M-s o' lists the lines of this buffer matching a pattern; `M-s g' searches
;;; a directory tree. Either way the matches land in a buffer of their own, and
;;; `M-g n' and `M-g p' walk them from wherever you are working.
;;;
;;; # What is here and what is not
;;;
;;; Nothing here searches anything. `occur--scan' and `grep--scan' find the
;;; matches, and what they hand back is not text but a *result set* attached to
;;; the listing buffer: one entry per match, each saying what it is in, which
;;; line, which column, and the text of that line. `results-entry',
;;; `results-visit' and `next-error' read that set.
;;;
;;; So this module is the view, and only the view: which window the listing
;;; opens in, what its keys do, and how the two marks look. A different view --
;;; a floating window, a list grouped by file, a tree -- replaces this file and
;;; changes none of the answers, because it asks the same questions.
;;;
;;; # Why the navigation keys are not defined here
;;;
;;; `next-error' and `previous-error' are built in. A compilation, a search and
;;; anything else that produces a list of places all attach one and are walked
;;; by the same two keys -- and none of them has to be loaded for another to
;;; work. A module that owned those keys would be a module every other producer
;;; had to depend on.

(make-mode 'occur-mode)

(defconst occur-window-height 12
  "How tall the window showing a listing is.

A strip across the bottom, like a compilation: the list is something you glance
at while working in the file above it, and the file is what deserves the room.")

;; ---------------------------------------------------------------------------
;; Reading the listing
;; ---------------------------------------------------------------------------

(defconst occur-header-lines 1
  "How many lines of the listing come before the first entry.

`occur--scan' writes one: the pattern and what was searched. Entry N is
therefore on line N + this + 1, and `occur--index-here' is the only place that
arithmetic is written down.")

(defun occur--index-here ()
  "Which entry the cursor's line names, or nil if it names none.

Counted rather than parsed. The line says `file:12: text', and reading it back
would be a second, worse copy of what the entry already knows -- worse because
a path with a colon in it parses two ways and the entry never did."
  (let ((index (- (line-number-at-point) occur-header-lines 1)))
    (if (and (>= index 0) (< index (or (results-count) 0)))
        index
        nil)))

(defcommand occur-goto () nil
  "Go to what the cursor's line found. Bound to RET in a listing."
  (let ((index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (results-visit index))))

(defcommand occur-show () nil
  "Show what the cursor's line found without leaving the listing.

For reading down a list of forty matches: the file follows the cursor, and the
cursor stays where it can be moved again."
  (let ((listing (current-buffer))
        (index (occur--index-here)))
    (if (null index)
        (message "No result on this line")
        (progn (results-visit index)
               (switch-to-buffer listing)))))

;; ---------------------------------------------------------------------------
;; Starting one
;; ---------------------------------------------------------------------------

(defun occur--display (buffer)
  "Show BUFFER as a strip across the bottom and go to it."
  (display-buffer-at-bottom buffer occur-window-height)
  (switch-to-buffer buffer)
  (goto-line 1)
  buffer)

(defcommand occur (pattern) ("sList lines matching: ")
  "List every line of this buffer matching PATTERN, in a buffer of its own.

The listing is a result set, so `M-g n' walks it from anywhere. `RET' on a line
goes to it, `o' shows it without leaving the list.

PATTERN is matched literally; `occur-regexp' is the same command for a regular
expression."
  (let ((buffer (occur--scan pattern nil)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand occur-regexp (pattern) ("sList lines matching regexp: ")
  "List every line of this buffer matching the regular expression PATTERN."
  (let ((buffer (occur--scan pattern t)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep (pattern) ("sSearch the tree for: ")
  "Search every file under `default-directory' for PATTERN and list what turns up.

The search runs in the background: the listing appears at once and fills as
files are read, with a `--- 12 matches ---' line when it is done. Killing the
listing stops it.

Large generated directories -- `.git', `target', `node_modules' -- are not
searched, and a file open with unsaved changes is searched as you have it
rather than as the disk has it."
  (let ((buffer (grep--scan pattern nil nil)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep-regexp (pattern) ("sSearch the tree for regexp: ")
  "Like `grep', with PATTERN read as a regular expression."
  (let ((buffer (grep--scan pattern nil t)))
    (if (null buffer)
        (message "Nothing to search")
        (occur--display buffer))))

(defcommand grep-in (directory) ("fSearch which directory: ")
  "Search DIRECTORY for a pattern, asking for the pattern next."
  (minibuffer-read "Search for:"
                   (lambda (pattern)
                     (unless (string= pattern "")
                       (let ((buffer (grep--scan pattern directory nil)))
                         (if buffer (occur--display buffer)))))
                   nil
                   nil))

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(define-key nil "M-s o" 'occur)
(define-key nil "M-s g" 'grep)

(define-key 'occur-mode "<ret>" 'occur-goto)
(define-key 'occur-mode "o" 'occur-show)
(define-key 'occur-mode "n" 'next-line)
(define-key 'occur-mode "p" 'previous-line)
(define-key 'occur-mode "q" 'delete-window)

;; The header and the trailer, so the shape of the list reads at a glance.
(add-syntax-rule 'occur-mode "^--- .* ---$" 'comment)
;; What each line is in, up to the colon before the line number. A path may
;; contain a colon; the *last* colon-number-colon is the one this module wrote.
(add-syntax-rule 'occur-mode "^(.*):[0-9]+: " 'type 1)

;; The two marks, as a compilation has them: backgrounds rather than
;; foregrounds, because the text under them is already coloured and a
;; background says "you are here" without arguing with what the text is.
;;
;; Naming them is also what turns them on: the marks are put on by the built-in
;; navigation, which looks the faces up by name and does nothing at all when
;; nothing has named them. A view that wants no marks simply does not name
;; these.
(set-face 'results-here nil "bright-black")
(set-face 'results-target nil "bright-black")

(log "occur loaded")
