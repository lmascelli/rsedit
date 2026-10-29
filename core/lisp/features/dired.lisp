;;; dired --- a directory in a buffer.
;;;
;;; A file manager written as a module: a mode, a keymap and a handful of
;;; commands, over primitives that know nothing of file managers -- `list-dir',
;;; `delete-file', `rename-file'. User manual: the `dired' page.
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
  (let ((line (current-line)))
    (if (string-prefix-p "  " line)
        (substring line 2)
        nil)))

(defun dired--path-here ()
  "The full path of the entry on point's line, or nil; a directory keeps its
trailing slash."
  (let ((name (dired--name-here)))
    (if name (concat (dired--directory) name) nil)))

;; ---------------------------------------------------------------------------
;; Showing a directory
;; ---------------------------------------------------------------------------

(defun dired--draw (directory)
  "Replace the listing with one of DIRECTORY."
  ;; Writable only while it is written.
  (set-buffer-read-only nil)
  (clear-buffer)
  (insert directory ":\n")
  ;; `..' listed, which `list-dir' leaves out: a listing shows its way out.
  (insert "  ..\n")
  ;; `list-dir' sorts: it has already asked what each entry is.
  (mapc (lambda (entry) (insert "  " entry "\n"))
        (list-dir directory dired-sort))
  (set-buffer-read-only t))

(defun dired--show (directory)
  "Show a listing of DIRECTORY in the dired buffer, and go to its first entry."
  ;; Expanded: a relative path in the header has no parent to work out.
  (let ((directory (file-name-as-directory (expand-file-name directory))))
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
   ((string= "  .." (current-line)) (dired-up-directory))
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
;; Both ask for a typed answer, since a file cannot be given back; the
;; callback closes over the path, kept in no variable.

(defun dired--affirmative (answer)
  "Whether ANSWER is a yes. Anything else, including empty, is a no."
  (or (string= "yes" (downcase answer)) (string= "y" (downcase answer))))

(defun dired-delete-file ()
  "Delete what the cursor is on, after asking; a directory goes with
everything in it, and the question says how many entries that is."
  (let ((path (dired--path-here)))
    (cond
     ((null path) (message "No file on this line"))
     ((string= "  .." (current-line)) (message "Refusing to delete .."))
     (t (let* ((directory (file-directory-p path))
               (count (if directory (directory-entry-count path) 0))
               ;; Settled with the wording: the answer is to the question
               ;; shown.
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
  "Report the result of deleting PATH, and read the listing again."
  (if outcome
      (message "Deleted %s" path)
      (message "Could not delete %s -- see M-x switch-to-messages" path))
  (dired-refresh))

(defun dired-rename-file ()
  "Rename what the cursor is on, asking for the new name: a path in it is
taken from the directory being listed."
  (let ((path (dired--path-here))
        ;; Read now: in the callback the current buffer may be another.
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
  "Where a rename to NEW-NAME lands, for a file listed in DIRECTORY."
  ;; Against the listing on screen, not the working directory, which is not
  ;; shown.
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
;; What the mode does not bind falls through to the global keymap; the
;; buffer is read-only, so nothing needs unbinding.

(define-key 'dired-mode "<ret>" 'dired-find-file)
(define-key 'dired-mode "o" 'dired-find-file-other-window)
(define-key 'dired-mode "^" 'dired-up-directory)
(define-key 'dired-mode "g" 'dired-refresh)
(define-key 'dired-mode "D" 'dired-delete-file)
(define-key 'dired-mode "R" 'dired-rename-file)

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

;; The header and the directories, coloured.
(add-syntax-rule 'dired-mode "^[^ ].*:$" 'keyword)
(add-syntax-rule 'dired-mode "^  (.*/)$" 'function 1)

(log "End of the dired.lisp")
(provide 'dired)
