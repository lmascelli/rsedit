;;; dired --- a directory in a buffer.
;;;
;;; A file manager written as a module: a mode, a keymap and a handful of
;;; commands, over primitives that know nothing of file managers -- `list-dir',
;;; `delete-file', `rename-file', `copy-file'. Files are marked, as in Emacs,
;;; and the commands act on the marked ones, or on the one under the cursor.
;;; User manual: the `dired' page.
;;; Design: `doc/architettura-editor.typ, <mod-dired>`.

(require 'commands)
(require 'debug)

(make-mode 'dired-mode)

(defconst dired-buffer-name "*dired*"
  "The buffer every listing is shown in.")

(defvar dired-sort 'name
  "How a listing is ordered: the symbol name for one alphabetical run, type
for the directories first and then the files.")

;; ---------------------------------------------------------------------------
;; Paths
;; ---------------------------------------------------------------------------

(defun dired--parent (directory)
  "The directory above DIRECTORY, or the root at the root."
  ;; Asked of the platform, not split on "/"; expanded first, since "./" has
  ;; no component to drop.
  (file-name-directory (directory-file-name (expand-file-name directory))))

;; ---------------------------------------------------------------------------
;; Reading the buffer back
;; ---------------------------------------------------------------------------

(defun dired--directory ()
  "The directory this listing is of, read from its first line; point stays."
  (let ((here (point)))
    (goto-line 1)
    (let ((header (current-line)))
      (goto-char here)
      ;; The header ends in the `:' that marks it as one.
      (file-name-as-directory (substring header 0 -1)))))

(defun dired--name-here ()
  "The name on point's line, or nil when the line is not an entry."
  ;; An entry is a mark, or a blank, then a blank, then its name; the header,
  ;; on the first line, is the directory.
  (let ((line (current-line)))
    (if (and (> (line-number-at-point) 1)
             (> (length line) 2)
             (string= (substring line 1 2) " "))
        (substring line 2)
        nil)))

(defun dired--parent-entry-p ()
  "Whether point's line is the `..' entry."
  (equal (dired--name-here) ".."))

(defun dired--path-here ()
  "The full path of the entry on point's line, or nil; a directory keeps its
trailing slash."
  (let ((name (dired--name-here)))
    (if name (concat (dired--directory) name) nil)))

;; ---------------------------------------------------------------------------
;; Showing a directory
;; ---------------------------------------------------------------------------

;; ---------------------------------------------------------------------------
;; Marks
;; ---------------------------------------------------------------------------
;;
;; A mark is the first column of an entry's line, as in Emacs: * for marked,
;; D for flagged for deletion. They are kept by path as well, so that a
;; listing read again from the disk shows them where they were, and lost
;; with the directory when another is listed.

(defconst dired-marker-char "*"
  "The mark `m' puts on a file, for the commands that act on marked files.")

(defconst dired-del-marker "D"
  "The mark `d' puts on a file, for `x' to delete.")

;; The marks of the directory listed, as (PATH . MARK).
(setq dired--marks nil)

(defun dired--mark-of (path)
  "The mark on PATH, or nil."
  (let ((found (assoc path dired--marks)))
    (if found (cdr found) nil)))

(defun dired--forget-mark (path)
  "Take PATH's mark away, if it has one."
  (let ((kept nil))
    (mapc (lambda (pair) (unless (equal (car pair) path) (setq kept (cons pair kept))))
          dired--marks)
    (setq dired--marks (reverse kept))))

(defun dired--set-mark (mark)
  "Put MARK, a one-character string or nil for none, on point's entry: in the
first column of its line and in `dired--marks'. Point stays on the line."
  (let ((path (dired--path-here)))
    (dired--forget-mark path)
    (if mark (setq dired--marks (append dired--marks (list (cons path mark)))))
    (set-buffer-read-only nil)
    (beginning-of-line)
    (delete-char 1)
    (insert (if mark mark " "))
    (beginning-of-line)
    (set-buffer-read-only t)))

(defun dired--marked (mark)
  "The paths marked with MARK, in the order the listing shows them."
  (let ((found nil))
    (mapc (lambda (pair) (if (equal (cdr pair) mark) (setq found (cons (car pair) found))))
          dired--marks)
    (dired--in-listing-order (reverse found))))

(defun dired--in-listing-order (paths)
  "PATHS, those still listed, in the order the listing shows them."
  (let ((directory (dired--directory))
        (out nil))
    (mapc (lambda (entry)
            (let ((path (concat directory entry)))
              (if (member path paths) (setq out (cons path out)))))
          (list-dir directory dired-sort))
    (reverse out)))

(defun dired--targets ()
  "What a command acts on: the files marked with *, or else the one under the
cursor, as a list of paths; nil when there is neither."
  (let ((marked (dired--marked dired-marker-char)))
    (cond
     (marked marked)
     ((or (null (dired--path-here)) (dired--parent-entry-p)) nil)
     (t (list (dired--path-here))))))

(defun dired--mark-and-go-on (mark)
  "Put MARK on point's entry and go to the next line, as m, d and u do."
  (cond
   ((null (dired--path-here)) (message "No file on this line"))
   ((dired--parent-entry-p) (message "Cannot mark .."))
   (t (dired--set-mark mark)))
  (next-line))

(defcommand dired-mark () nil
  "Mark the file under the cursor for the commands that act on marked files,
and go to the next line."
  (dired--mark-and-go-on dired-marker-char))

(defcommand dired-flag-file-deletion () nil
  "Flag the file under the cursor for deletion by `x', and go to the next line."
  (dired--mark-and-go-on dired-del-marker))

(defcommand dired-unmark () nil
  "Take the mark off the file under the cursor, and go to the next line."
  (dired--mark-and-go-on nil))

(defcommand dired-unmark-backward () nil
  "Go to the line above and take the mark off its file."
  (previous-line)
  (if (and (dired--path-here) (not (dired--parent-entry-p)))
      (dired--set-mark nil)))

(defcommand dired-unmark-all-marks () nil
  "Take every mark and every deletion flag off the listing."
  (let ((count (length dired--marks)))
    (setq dired--marks nil)
    (dired-refresh)
    (message "%s mark%s removed" count (if (= count 1) "" "s"))))

(defcommand dired-toggle-marks () nil
  "Mark the files that are not marked and unmark those that are; files flagged
for deletion stay flagged."
  (let ((directory (dired--directory))
        (kept nil))
    (mapc (lambda (entry)
            (let* ((path (concat directory entry))
                   (mark (dired--mark-of path)))
              (cond
               ((equal mark dired-del-marker) (setq kept (cons (cons path mark) kept)))
               ((equal mark dired-marker-char) nil)
               (t (setq kept (cons (cons path dired-marker-char) kept))))))
          (list-dir directory dired-sort))
    (setq dired--marks (reverse kept))
    (dired-refresh)))

(defun dired--draw (directory)
  "Replace the listing with one of DIRECTORY, each entry with its mark."
  ;; Writable only while it is written.
  (set-buffer-read-only nil)
  (clear-buffer)
  ;; The listing's own directory, as Emacs' default-directory is: a relative
  ;; name, or a shell command run from here, is read in it.
  (set-buffer-directory directory)
  (insert directory ":\n")
  ;; `..' listed, which `list-dir' leaves out: a listing shows its way out.
  (insert "  ..\n")
  ;; `list-dir' sorts: it has already asked what each entry is.
  (mapc (lambda (entry)
          (let ((mark (dired--mark-of (concat directory entry))))
            (insert (if mark mark " ") " " entry "\n")))
        (list-dir directory dired-sort))
  (set-buffer-read-only t))

(defun dired--show (directory)
  "Show a listing of DIRECTORY in the dired buffer, and go to its first entry."
  ;; Expanded: a relative path in the header has no parent to work out.
  (let ((directory (file-name-as-directory (expand-file-name directory))))
    ;; Marks belong to the directory they were made in.
    (if (not (and (member dired-buffer-name (all-buffer-names))
                  (equal directory
                         (with-current-buffer dired-buffer-name
                           (lambda () (dired--directory))))))
        (setq dired--marks nil))
    (buffer-create dired-buffer-name 'dired-mode)
    (switch-to-buffer dired-buffer-name)
    (dired--draw directory)
    ;; Line 2, `..', the first line a key can act on.
    (goto-line 2)))

(defcommand dired (directory) ("fDired: ")
  "List DIRECTORY in a buffer where keys open, delete and rename its files."
  (if (file-directory-p directory)
      (dired--show directory)
      (message "%s is not a directory" directory)))

(defcommand dired-refresh () nil
  "Read the directory being listed again, keeping the cursor on the same line."
  ;; Every change ends here: rebuilt from the disk, not edited to match.
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
  "Open PATH: a directory as the listing, a file in this window."
  ;; `file-directory-p' decides, not the separator, which belongs to the
  ;; platform; `..' goes up, so the header does not grow a `..'.
  (cond
   ((dired--parent-entry-p) (dired-up-directory))
   ((file-directory-p path) (dired--show path))
   (t (find-file path))))

(defcommand dired-find-file () nil
  "Open what the cursor is on, in this window."
  (let ((path (dired--path-here)))
    (if path
        (dired--visit path)
        (message "No file on this line"))))

;; Columns, not a share of the frame: a listing is as wide as its names.
(defvar dired-window-width 40
  "How many columns the listing keeps when `o' opens a file beside it.")

;; The window `o' last opened a file into, which it opens the next into; nil
;; before the first `o', or once that window is closed.
(setq dired--file-window nil)

(defcommand dired-find-file-other-window () nil
  "Open what the cursor is on in a window beside the listing, the one the last
`o' opened when it is still there."
  (let ((path (dired--path-here)))
    (cond
     ((null path) (message "No file on this line"))
     ;; A directory opens here: there is one listing buffer.
     ((file-directory-p path) (dired--visit path))
     (t (dired--visit-other-window path)))))

(defun dired--visit-other-window (path)
  "Open PATH beside the listing, in the window `o' last used if it is there."
  ;; `select-window' answers nil for a window closed since.
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
;; Every command here acts on the files marked with *, or on the one under
;; the cursor when none is, and `x' on those flagged with D. Deleting asks
;; with `yes-or-no', since a file cannot be given back: typed in full, or one
;; key with `use-short-answers'. The callbacks close over the paths, kept in
;; no variable.

(defun dired--file-count (paths)
  "How PATHS are spoken of in a question: its name for one, a count for more."
  (if (= (length paths) 1)
      (car paths)
      (format "%s files" (length paths))))

(defun dired--full-directories (paths)
  "The directories among PATHS that have something in them."
  (let ((found nil))
    (mapc (lambda (path)
            (if (and (file-directory-p path)
                     (let ((count (directory-entry-count path)))
                       (and count (> count 0))))
                (setq found (cons path found))))
          paths)
    (reverse found)))

(defun dired--delete-question (paths)
  "The question deleting PATHS asks, which says what would go with them."
  ;; Settled with the wording: the answer is to the question shown.
  (let ((full (dired--full-directories paths)))
    (cond
     ((and (= (length paths) 1) full)
      (format "Recursively delete %s (%s entries)?"
              (car paths) (directory-entry-count (car paths))))
     (full
      (format "Delete %s, %s of them directories with everything in them?"
              (dired--file-count paths) (length full)))
     (t (format "Delete %s?" (dired--file-count paths))))))

(defun dired--delete (paths)
  "Ask, then delete PATHS, directories with everything in them."
  (yes-or-no (dired--delete-question paths)
             (lambda () (dired--deleted paths))
             (lambda () (message "Not deleted"))))

(defun dired--deleted (paths)
  "Delete PATHS, report how it went, and read the listing again."
  (let ((failed nil))
    (mapc (lambda (path)
            (if (delete-file path t)
                (dired--forget-mark path)
                (setq failed (cons path failed))))
          paths)
    (cond
     (failed
      (message "Could not delete %s -- see M-x switch-to-messages"
               (dired--file-count (reverse failed))))
     ((= (length paths) 1) (message "Deleted %s" (car paths)))
     (t (message "Deleted %s files" (length paths)))))
  (dired-refresh))

(defcommand dired-do-delete () nil
  "Delete the marked files, or the one under the cursor, after asking once; a
directory goes with everything in it, and the question says so."
  (let ((paths (dired--targets)))
    (cond
     ((dired--parent-entry-p) (message "Refusing to delete .."))
     ((null paths) (message "No file on this line"))
     (t (dired--delete paths)))))

(defcommand dired-do-flagged-delete () nil
  "Delete the files flagged with D, after asking once."
  (let ((paths (dired--marked dired-del-marker)))
    (if paths
        (dired--delete paths)
        (message "No files flagged for deletion"))))

;; The command `D' ran before marks: kept, as what it always meant.
(defun dired-delete-file ()
  "Delete what the cursor is on, or the marked files, after asking."
  (dired-do-delete))

(defun dired--destination (directory new-name)
  "Where a rename or a copy to NEW-NAME lands, for a file listed in
DIRECTORY."
  ;; Against the listing on screen, not the working directory, which is not
  ;; shown.
  (expand-file-name new-name directory))

(defun dired--into (path directory)
  "Where PATH goes when it is moved or copied into DIRECTORY: its own name
there."
  (concat (file-name-as-directory directory)
          (file-name-nondirectory (directory-file-name path))))

(defun dired--transfer (verb paths directory action)
  "Ask where PATHS go, then do ACTION, a function of the path and where it
goes that answers t or nil, for each. VERB names it in the question. One
path may go to a new name, or into a directory; more go into a directory."
  (let ((one (= (length paths) 1)))
    (minibuffer-read
     (if one
         (format "%s %s to:" verb (file-name-nondirectory (directory-file-name (car paths))))
         (format "%s %s files into directory:" verb (length paths)))
     (lambda (answer)
       (let ((target (dired--destination directory answer)))
         (cond
          ((string= answer "") (message "Nothing done"))
          ((and (not one) (not (file-directory-p target)))
           (message "%s is not a directory" target))
          (t (dired--transferred
              verb paths
              (mapcar (lambda (path)
                        (funcall action path
                                 (if (file-directory-p target)
                                     (dired--into path target)
                                     target)))
                      paths))))))
     nil nil)))

(defun dired--transferred (verb paths outcomes)
  "Report OUTCOMES, one t or nil for each of PATHS, which VERB, Copy or
Rename, acted on, and read the listing again."
  (let ((done-verb (if (string= verb "Copy") "Copied" "Renamed"))
        (failed 0) (done 0))
    (mapc (lambda (outcome)
            (if outcome (setq done (+ done 1)) (setq failed (+ failed 1))))
          outcomes)
    (cond
     ((and (= (length paths) 1) (= failed 1))
      (message "Could not %s %s -- see M-x switch-to-messages" (downcase verb) (car paths)))
     ((= (length paths) 1) (message "%s %s" done-verb (car paths)))
     ((> failed 0)
      (message "%s %s files, %s failed -- see M-x switch-to-messages" done-verb done failed))
     (t (message "%s %s files" done-verb done))))
  (dired-refresh))

(defcommand dired-do-rename () nil
  "Rename the file under the cursor, asking for the new name, or move the
marked files into a directory; a relative name is taken from the directory
being listed."
  (let ((paths (dired--targets))
        ;; Read now: in the callback the current buffer may be another.
        (directory (dired--directory)))
    (cond
     ((dired--parent-entry-p) (message "Refusing to rename .."))
     ((null paths) (message "No file on this line"))
     (t (dired--transfer "Rename" paths directory
                         (lambda (path to)
                           (let ((outcome (rename-file path to)))
                             (if outcome (dired--forget-mark path))
                             outcome)))))))

;; The command `R' ran before marks: kept, as what it always meant.
(defun dired-rename-file ()
  "Rename what the cursor is on, or move the marked files, asking where."
  (dired-do-rename))

(defcommand dired-do-copy () nil
  "Copy the file under the cursor to a name asked for, or the marked files
into a directory; directories are copied with everything in them. An
existing file is never replaced."
  (let ((paths (dired--targets))
        (directory (dired--directory)))
    (cond
     ((dired--parent-entry-p) (message "Refusing to copy .."))
     ((null paths) (message "No file on this line"))
     (t (dired--transfer "Copy" paths directory
                         (lambda (path to) (copy-file path to t)))))))

(defun dired--shell-line (command paths)
  "COMMAND with PATHS, quoted for the shell, put where a * standing alone in
it is, or else after it."
  (let ((quoted ""))
    (mapc (lambda (path)
            (setq quoted (concat quoted (if (string= quoted "") "" " ")
                                 (shell-quote-argument path))))
          paths)
    (if (string-match "(^|[ \t])\\*([ \t]|$)" command)
        ;; The blanks either side of the star are kept around the files.
        (concat (substring command 0 (match-beginning 0))
                (match-string 1 command) quoted (match-string 2 command)
                (substring command (match-end 0)))
        (concat command " " quoted))))

(defcommand dired-do-shell-command () nil
  "Run a shell command on the marked files, or the one under the cursor, in
the directory being listed: a * standing alone in the command is where the
files go, and without one they go after it. What it prints is shown as by
`shell-command'."
  (let ((paths (dired--targets))
        (directory (dired--directory)))
    (cond
     ((dired--parent-entry-p) (message "Refusing to run a command on .."))
     ((null paths) (message "No file on this line"))
     (t (minibuffer-read
         (format "! on %s:" (dired--file-count
                             (mapcar (lambda (path)
                                       (file-name-nondirectory (directory-file-name path)))
                                     paths)))
         (lambda (command)
           (if (string= command "")
               (message "No command")
               ;; From the listing, whose own directory it starts in; the
               ;; callback may run with another buffer current.
               (with-current-buffer dired-buffer-name
                 (lambda () (shell-command (dired--shell-line command paths))))))
         nil nil)))))

;; ---------------------------------------------------------------------------
;; Keys
;; ---------------------------------------------------------------------------
;;
;; What the mode does not bind falls through to the global keymap; the
;; buffer is read-only, so nothing needs unbinding.

(define-key 'dired-mode "<ret>" 'dired-find-file)
(define-key 'dired-mode "o" 'dired-find-file-other-window)
(define-key 'dired-mode "^" 'dired-up-directory)
(define-key 'dired-mode "g" 'dired-refresh)
(define-key 'dired-mode "D" 'dired-do-delete)
(define-key 'dired-mode "R" 'dired-do-rename)
(define-key 'dired-mode "C" 'dired-do-copy)
(define-key 'dired-mode "!" 'dired-do-shell-command)
(define-key 'dired-mode "m" 'dired-mark)
(define-key 'dired-mode "d" 'dired-flag-file-deletion)
(define-key 'dired-mode "u" 'dired-unmark)
(define-key 'dired-mode "<backspace>" 'dired-unmark-backward)
(define-key 'dired-mode "U" 'dired-unmark-all-marks)
(define-key 'dired-mode "t" 'dired-toggle-marks)
(define-key 'dired-mode "x" 'dired-do-flagged-delete)

(define-key 'dired-mode "n" 'next-line)
(define-key 'dired-mode "p" 'previous-line)
(define-key 'dired-mode "q" '(close-buffer dired-buffer-name))


;; ---------------------------------------------------------------------------
;; Finding a file anywhere below here
;; ---------------------------------------------------------------------------
;;
;; The walk is a primitive, `directory-files-recursive', which prunes as it
;; goes: `.git' alone would exhaust its limit.

(defvar find-file-recursive-ignore
  '(".git" ".hg" ".svn" ".jj"
    "target" "build" "dist" "out"
    "node_modules" "vendor" ".venv" "venv" "__pycache__"
    ".cache" ".mypy_cache" ".pytest_cache" ".tox")
  "Directory names `dired-find-recursive' never goes into, at any depth.")

(defvar find-file-recursive-limit 5000
  "How many files `dired-find-recursive' collects before it stops, saying so.")

;; ---------------------------------------------------------------------------
;; .gitignore, approximately
;; ---------------------------------------------------------------------------
;;
;; Approximate on purpose: a pattern not understood is skipped, so a file git
;; would hide may be offered, but none it would show is hidden.

(defun find-file-recursive--gitignore-lines (directory)
  "The meaningful lines of DIRECTORY's .gitignore, or nil if it has none."
  (let ((text (read-file-to-string (concat (file-name-as-directory directory)
                                           ".gitignore"))))
    (if (null text)
        nil
        ;; Blank lines and comments dropped; consed and reversed, since
        ;; `append' would copy the list for every line.
        (let ((kept nil))
          (mapc (lambda (line)
                  (let ((trimmed (find-file-recursive--trim line)))
                    (if (and (not (string= trimmed ""))
                             (not (string= (substring trimmed 0 1) "#")))
                        (setq kept (cons trimmed kept)))))
                (split-string text "\n"))
          (reverse kept)))))

(defun find-file-recursive--trim (text)
  "TEXT without leading or trailing spaces, tabs or carriage returns."
  ;; A .gitignore written on Windows ends every line with a carriage return.
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
  "Directory names DIRECTORY's .gitignore asks to leave out."
  ;; A plain name is a file or a directory to git; read as a directory here.
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
  "The endings DIRECTORY's .gitignore asks to leave out, from its *.ending lines."
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
  "Every file under DIRECTORY worth offering, as (TRUNCATED PATHS)."
  ;; Both exclusions go to the walk: filtering after would cost an evaluation
  ;; per file per pattern.
  (directory-files-recursive directory
                             find-file-recursive-limit
                             (append find-file-recursive-ignore
                                     (find-file-recursive--ignored-directories directory))
                             (find-file-recursive--ignored-suffixes directory)))

(defun dired-find-recursive--candidates (input)
  "The paths found when the prompt opened that match INPUT, best first."
  ;; One walk, narrowed on every key, not a walk per key.
  (fuzzy-filter input dired--recursive-candidates))

(defcommand dired-find-recursive () nil
  "Open a file from anywhere under the working directory, choosing it by any
part of its path."
  (let* ((found (find-file-recursive--candidates "."))
         (truncated (car found))
         (paths (nth 1 found)))
    (cond
     ((null paths) (message "No files under %s" (expand-file-name ".")))
     (t (progn
          ;; Held for the life of the prompt.
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

;; The list the prompt narrows, emptied when it is answered.
(setq dired--recursive-candidates nil)

(define-key nil "C-x d" 'dired)
(define-key nil "C-x C-r" 'dired-find-recursive)

;; What `find-file' hands a directory to.
(setq *open-directory-callback* 'dired)

;; The header and the directories, coloured, and the marked and flagged
;; entries: the header is the one line that does not start with a mark, or a
;; blank, and then a blank.
(add-syntax-rule 'dired-mode "^(?:[^ *D]|[*D][^ ]).*:$" 'keyword)
(add-syntax-rule 'dired-mode "^. (.*/)$" 'function 1)
(define-face 'dired-marked "yellow" nil '("bold"))
(define-face 'dired-flagged "red" nil '("bold"))
(add-syntax-rule 'dired-mode "^\\* .*$" 'dired-marked)
(add-syntax-rule 'dired-mode "^D .*$" 'dired-flagged)

(log "End of the dired.lisp")
(provide 'dired)
