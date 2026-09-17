;;; find-file-recursive --- open any file under the current directory by name.
;;;
;;; `C-x C-f' asks for a path one directory at a time, which is the right thing
;;; when you know where the file is. This is for when you know what it is
;;; called: it lists every file under the working directory at once and lets
;;; you pick from the completion strip.
;;;
;;; The walk itself is `directory-files-recursive', a primitive, because it has
;;; to be. Everything about *which* files are worth listing is here, in Lisp,
;;; because it is policy and policy changes per project.
;;;
;;; One exception, and it is worth understanding: the ignore list is passed
;;; *into* the walk rather than applied to its result. Filtering afterwards
;;; cannot work. `.git' in a working repository holds thousands of objects, and
;;; a walk that collected them would hit its limit before reaching a single
;;; source file -- the files you wanted would not be in the list to filter.

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
        (let ((kept nil))
          (mapc (lambda (line)
                  (let ((trimmed (find-file-recursive--trim line)))
                    (if (and (not (string= trimmed ""))
                             (not (string= (substring trimmed 0 1) "#")))
                        (setq kept (append kept (list trimmed))))))
                (split-string text "\n"))
          kept))))

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
      (setq out (append out (list (substring text n (+ n 1)))))
      (setq n (+ n 1)))
    out))

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
                (setq suffixes (append suffixes (list (substring pattern 1))))))
          (find-file-recursive--gitignore-lines directory))
    suffixes))

(defun find-file-recursive--has-suffix (path suffixes)
  "Whether PATH ends with any of SUFFIXES."
  (let ((found nil))
    (mapc (lambda (suffix)
            (if (and (>= (length path) (length suffix))
                     (string= (substring path (- (length path) (length suffix)))
                              suffix))
                (setq found t)))
          suffixes)
    found))

;; ---------------------------------------------------------------------------
;; The command
;; ---------------------------------------------------------------------------

(defun find-file-recursive--candidates (directory)
  "Every file under DIRECTORY worth offering, as (TRUNCATED PATHS)."
  (let* ((pruned (append find-file-recursive-ignore
                         (find-file-recursive--ignored-directories directory)))
         (suffixes (find-file-recursive--ignored-suffixes directory))
         (walked (directory-files-recursive directory
                                            find-file-recursive-limit
                                            pruned))
         (truncated (car walked))
         (kept nil))
    (mapc (lambda (path)
            (if (not (find-file-recursive--has-suffix path suffixes))
                (setq kept (append kept (list path)))))
          (nth 1 walked))
    (list truncated kept)))

(defcommand find-file-recursive () nil
  "Open a file from anywhere under the working directory, choosing it by name.

Where `C-x C-f' walks the tree a directory at a time, this flattens it: every
file under the current directory is offered at once, and you pick from the
completion strip. `.git', `target', `node_modules' and the rest of
`find-file-recursive-ignore' are never descended into, and the working
directory's .gitignore is read for more.

If the search stops at `find-file-recursive-limit' it says so. That matters --
a truncated list looks exactly like a complete one, and a file missing from it
looks exactly like a file that is not there."
  (let* ((found (find-file-recursive--candidates "."))
         (truncated (car found))
         (paths (nth 1 found)))
    (cond
     ((null paths) (message "No files under %s" (expand-file-name ".")))
     (t (progn
          (if truncated
              (message "%s files (truncated at %s -- narrow the directory or raise find-file-recursive-limit)"
                       (length paths) find-file-recursive-limit))
          (funcall *completion-read-function*
                   paths
                   (lambda (path) (find-file path))))))))

(define-key nil "C-x C-r" 'find-file-recursive)

(log "find-file-recursive loaded")
