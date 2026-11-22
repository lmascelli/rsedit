;;; project --- the directory a file belongs to, and commands that work there.
;;;
;;; A project is a root directory, found above the current file by the names
;;; of `project-markers' or declared by hand; C-x p finds a file, searches,
;;; builds and runs a command there, and C-x p p goes to a project known from
;;; an earlier run. It knows its buffers -- its files, what was started in
;;; it, its shell, and those named by `project-buffer-name-rules' -- and C-x p
;;; b, e, k and d work on them. User manual: the `project' page.
;;; Design: `doc/architettura-editor.typ, <mod-project>`.

(require 'commands)
(require 'debug)
(require 'dired)
(require 'occur)
(require 'shell)
(require 'compile)
(require 'eshell)

;; ---------------------------------------------------------------------------
;; Settings
;; ---------------------------------------------------------------------------

(defvar project-markers
  '(".git"
    (".hg" ".jj" ".svn")
    ("Cargo.toml" "package.json" "pyproject.toml" "go.mod"
     "CMakeLists.txt" "Makefile" ".project"))
  "The names that make the directory holding one a project root: a list of
entries, each a name or a list of names. The first entry found above a
directory decides, and within an entry the nearest directory wins, so a
repository's .git wins over a nearer Cargo.toml.")

(defvar project-roots nil
  "Directories that are project roots whatever they hold, as
`project-declare-root' declares them. The nearest above a directory wins over
every marker.")

(defvar project-buffer-name-rules '("^\\*%p-")
  "Regular expressions naming the buffers a project claims, in which %p
stands for the project's name, the last directory of its root. A buffer made
with no file from a buffer of a project, whose name one of them matches,
belongs to that project and has its root as directory, wherever it was made
from: by default every buffer whose name starts with *NAME-, as the
project's shell, *NAME-eshell*.")

(defvar project-buffer-ignore
  '("^ " "^\\*Minibuffer\\*$" "^\\*Completions\\*$" "^\\*scratch\\*$"
    "^\\*log\\*$" "^\\*Help\\*$" "^\\*Manual\\*$" "^\\*Buffer List\\*$"
    "^\\*Themes\\*$")
  "Regular expressions naming buffers that belong to no project, whatever
their directory: the editor's own, shared by every project.")

;; ---------------------------------------------------------------------------
;; Finding the root
;; ---------------------------------------------------------------------------

(defun project--directory (path)
  "PATH, absolute, ending in a separator."
  (file-name-as-directory (expand-file-name path)))

(defun project--start (directory)
  "Where the search for a root starts: DIRECTORY, the current buffer's file,
or the working directory."
  (cond (directory (expand-file-name directory))
        ((buffer-file-name) (buffer-file-name))
        (t (working-directory))))

(defun project--declared ()
  "Every root declared by hand: `project-roots', then those of the session."
  (append (mapcar 'project--directory project-roots)
          (session-value 'project-declared-roots)))

(defun project--declared-above (start)
  "The nearest declared root that START is in, or nil."
  (let ((inside (file-name-as-directory start))
        (best nil))
    (dolist (root (project--declared))
      (if (and (string-prefix-p root inside)
               (or (null best) (> (length root) (length best))))
          (setq best root)))
    best))

(defun project--marked-above (start)
  "The root the first entry of `project-markers' found above START gives, nil
when none is found, or timeout while the disk has not answered."
  (catch 'found
    (dolist (entry project-markers)
      (let ((root (locate-dominating-file start entry)))
        (if root (throw 'found root))))
    nil))

(defun project-root (&optional directory)
  "Return the root of the project DIRECTORY is in, ending in a separator: the
nearest root declared in `project-roots' or with `project-declare-root', or
else the one `project-markers' finds. nil when there is none, and the symbol
timeout when the disk has not answered yet. DIRECTORY is by default the
current buffer's: the project the buffer was claimed by, when it was, as the
project's shell is wherever it has gone; else its file, or its directory.

Example:
  (project-root \"~/src/app/src/main.rs\")"
  (let ((claimed (and (null directory) (buffer-get 'project-root))))
    (or claimed
        (let ((start (project--start directory)))
          (or (project--declared-above start)
              (project--marked-above start))))))

;; ---------------------------------------------------------------------------
;; The projects known
;; ---------------------------------------------------------------------------

(defun project--without (item list)
  "LIST without the elements equal to ITEM."
  (let ((kept nil))
    (dolist (element list)
      (unless (equal element item)
        (setq kept (cons element kept))))
    (reverse kept)))

(defun project--drop (root name)
  "Take ROOT out of the list the session keeps under NAME."
  (set-session-value name (project--without root (session-value name))))

(defun project--remember (root)
  "Put ROOT at the front of the projects known, and return it."
  (let ((known (session-value 'project-known)))
    (unless (equal root (car known))
      (set-session-value 'project-known
                         (cons root (project--without root known)))))
  root)

(defun project-known-projects ()
  "Return the roots of the projects known, newest first: those a project
command ran in, in this run or an earlier one, then those declared by hand."
  (let ((all (session-value 'project-known)))
    (dolist (root (project--declared))
      (unless (member root all)
        (setq all (append all (list root)))))
    all))

(defun project--current ()
  "The current project's root, remembered as known, or nil after saying why
there is none."
  (let ((root (project-root)))
    (cond ((stringp root) (project--remember root))
          ((eq root 'timeout)
           (message "The disk has not said yet where the project is: try again")
           nil)
          (t (message "Not in a project: M-x project-declare-root declares one")
             nil))))

(defun project--abbreviate (root)
  "ROOT as a prompt shows it: its last directory's name."
  (file-name-nondirectory (directory-file-name root)))

;; ---------------------------------------------------------------------------
;; The buffers of a project
;; ---------------------------------------------------------------------------
;;
;; A buffer belongs to the project its directory is in, as in Emacs: a file's
;; buffer to its file's, a listing or an output to the one it was made in. A
;; buffer claimed by a project -- its shell, one named by a rule -- keeps
;; `project-root' and belongs to it wherever its directory goes.

(defun project--quote (text)
  "TEXT with every character a regular expression gives a meaning to escaped."
  (let ((answer ""))
    (dotimes (i (length text))
      (let ((c (substring text i (+ i 1))))
        (setq answer (concat answer
                             (if (member c '("\\" "." "+" "*" "?" "(" ")" "["
                                             "]" "{" "}" "^" "$" "|"))
                                 "\\" "")
                             c))))
    answer))

(defun project-name (&optional root)
  "Return the name of the project at ROOT, the current one by default: the
last directory of its root. nil outside a project.

Example:
  (project-name \"~/src/rsedit/\")   ; => \"rsedit\""
  (let ((root (or root (project-root))))
    (if (stringp root) (project--abbreviate root) nil)))

(defun project--rule (rule name)
  "RULE, a pattern of `project-buffer-name-rules', with each %p made NAME."
  (let ((answer "")
        (at 0))
    (while (< at (length rule))
      (if (and (< (+ at 1) (length rule))
               (string= (substring rule at (+ at 2)) "%p"))
          (progn (setq answer (concat answer (project--quote name)))
                 (setq at (+ at 2)))
          (progn (setq answer (concat answer (substring rule at (+ at 1))))
                 (setq at (+ at 1)))))
    answer))

(defun project--named-by-rule-p (buffer root)
  "Whether one of `project-buffer-name-rules' gives BUFFER to ROOT."
  (let ((name (project--abbreviate root)))
    (catch 'found
      (dolist (rule project-buffer-name-rules)
        (if (string-match-p (project--rule rule name) buffer)
            (throw 'found t)))
      nil)))

(defun project--ignored-p (buffer)
  "Whether BUFFER belongs to no project: an editor's own, or a prompt's."
  (or (memq (buffer-role buffer) '(prompt completion))
      (catch 'found
        (dolist (pattern project-buffer-ignore)
          (if (string-match-p pattern buffer) (throw 'found t)))
        nil)))

(defun project--buffer-root (buffer)
  "The root of the project BUFFER belongs to, or nil: the one that claimed
it, or else the one of its file or its directory."
  (or (buffer-get 'project-root buffer)
      (let ((root (project-root (or (buffer-file-name buffer)
                                    (file-name-as-directory
                                     (buffer-directory buffer))))))
        (if (stringp root) root nil))))

(defun project-buffer-p (buffer &optional root)
  "Return t when the buffer BUFFER belongs to the project at ROOT, the
current one by default: claimed by it, named by one of
`project-buffer-name-rules', or with its file or its directory in it, and not
named by `project-buffer-ignore'.

Example:
  (project-buffer-p \"*rsedit-eshell*\")"
  (let ((root (or root (project-root))))
    (cond ((not (stringp root)) nil)
          ((not (member buffer (all-buffer-names))) nil)
          ((project--ignored-p buffer) nil)
          ((buffer-get 'project-root buffer)
           (equal (buffer-get 'project-root buffer) root))
          ((project--named-by-rule-p buffer root) t)
          (t (equal (project--buffer-root buffer) root)))))

(defun project-buffers (&optional root)
  "Return the names of the buffers of the project at ROOT, the current one by
default, as `project-buffer-p' decides; nil outside a project.

Example:
  (length (project-buffers))"
  (let ((root (or root (project-root)))
        (found nil))
    (when (stringp root)
      (dolist (buffer (all-buffer-names))
        (if (project-buffer-p buffer root)
            (setq found (cons buffer found)))))
    (reverse found)))

(defun project-claim-buffer (root &optional buffer)
  "Make BUFFER, the current one by default, a buffer of the project at ROOT
wherever its directory goes, and give it ROOT as directory when it has no
file. Return ROOT.

Example:
  (project-claim-buffer (project-root) \"*notes*\")"
  (let ((root (project--directory root))
        (buffer (or buffer (current-buffer))))
    (buffer-put 'project-root root buffer)
    (unless (buffer-file-name buffer)
      (with-current-buffer buffer
                           (lambda () (set-buffer-directory root))))
    root))

(defun project--claim-new-buffer ()
  "Run when a buffer is made: claim the buffer just made, with no file, for
the project it was made in, when one of `project-buffer-name-rules' names
it."
  (let ((buffer (current-buffer)))
    (when (and project-buffer-name-rules
               (null (buffer-file-name))
               (not (project--ignored-p buffer)))
      (let ((root (project-root)))
        (when (and (stringp root) (project--named-by-rule-p buffer root))
          (project-claim-buffer root buffer))))))

(add-hook nil "buffer-created-hook" 'project--claim-new-buffer)

(defun project--other-buffers (root)
  "The buffers of ROOT but the current one, best first."
  (let ((here (current-buffer))
        (found nil))
    (dolist (buffer (project-buffers root))
      (unless (equal buffer here)
        (setq found (cons buffer found))))
    (reverse found)))

;; The buffers the open buffer prompt offers, found when it opened: while it
;; is open the prompt is the current buffer.
(setq project--offered-buffers nil)

(defun project-switch-to-buffer--candidates (input)
  "The buffers of the prompt's project that match INPUT, best first."
  (fuzzy-filter input project--offered-buffers))

(defcommand project-switch-to-buffer () nil
  "Ask for one of the buffers of the current project, by any part of its name,
and switch to it."
  (let ((root (project--current)))
    (when root
      (setq project--offered-buffers (project--other-buffers root))
      (if (null project--offered-buffers)
          (message "No other buffer in %s" (project--abbreviate root))
          (progn
            (minibuffer-read (format "Switch to buffer in %s:"
                                     (project--abbreviate root))
                             (lambda (buffer)
                               (cond ((string= buffer "") nil)
                                     ((member buffer (all-buffer-names))
                                      (switch-to-buffer buffer))
                                     (t (message "No buffer %s" buffer))))
                             'project-switch-to-buffer--candidates
                             nil nil "project-switch-to-buffer"))))))

(defun project-eshell-buffer-name (&optional root)
  "Return the name of the shell buffer of the project at ROOT, the current one
by default: *NAME-eshell*, which `project-buffer-name-rules' claims."
  (format "*%s-eshell*" (project-name root)))

(defcommand project-eshell () nil
  "Switch to the shell of the current project, *NAME-eshell*, starting one at
the root when none is running. The shell stays a buffer of the project after a
cd in it elsewhere."
  (let ((root (project--current)))
    (when root
      (let ((buffer (project-eshell-buffer-name root)))
        (if (eshell-process buffer)
            (switch-to-buffer buffer)
            (with-directory root
                            (lambda ()
                              (unless (member buffer (all-buffer-names))
                                (buffer-create buffer :mode 'eshell-mode
                                               :role 'editing))
                              (project-claim-buffer root buffer)
                              (eshell--open nil buffer))))))))

(defcommand project-dired () nil
  "List the root of the current project, as `dired' does."
  (let ((root (project--current)))
    (if root (dired root))))

(defun project--kill (buffers)
  "Kill BUFFERS, asked about already, saying how many."
  (let ((count 0))
    (dolist (buffer buffers)
      (when (member buffer (all-buffer-names))
        (kill-buffer-without-saving buffer)
        (setq count (+ count 1))))
    (message "Killed %s buffers" count)))

(defcommand project-kill-buffers () nil
  "Kill every buffer of the current project, after asking; the buffers with
unsaved changes are asked about once more, all together."
  (let ((root (project--current)))
    (when root
      (let ((buffers (project-buffers root)))
        (if (null buffers)
            (message "No buffers in %s" (project--abbreviate root))
            (y-or-n
             (format "Kill %s buffers of %s?" (length buffers)
                     (project--abbreviate root))
             (lambda ()
               (let ((modified nil))
                 (dolist (buffer buffers)
                   (if (and (buffer-file-name buffer) (buffer-modified-p buffer))
                       (setq modified (cons buffer modified))))
                 (if (null modified)
                     (project--kill buffers)
                     (yes-or-no
                      (format "%s of them have unsaved changes. Kill those too?"
                              (length modified))
                      (lambda () (project--kill buffers))
                      (lambda ()
                        (project--kill
                         (project--without-all modified buffers)))))))))))))

(defun project--without-all (items list)
  "LIST without the elements in ITEMS."
  (let ((kept nil))
    (dolist (element list)
      (unless (member element items)
        (setq kept (cons element kept))))
    (reverse kept)))

;; ---------------------------------------------------------------------------
;; Finding a file
;; ---------------------------------------------------------------------------
;;
;; The index is asked again at each Tab, as `dired-find-recursive' asks it,
;; and leaves out what that one leaves out.

;; The root the open prompt finds files under.
(setq project--find-root nil)

(defun project--index (root)
  "The index of the files under ROOT worth offering, as (STATE PATHS)."
  (file-index root
              (mapcar (lambda (name) (concat name "/"))
                      find-file-recursive-ignore)
              find-file-recursive-ignore-files
              find-file-recursive-limit))

(defun project-find-file--candidates (input)
  "The files of the prompt's project that match INPUT, best first."
  (fuzzy-filter input (nth 1 (project--index project--find-root))))

(defun project--find-file-in (root)
  "Ask for a file under ROOT, by any part of its path, and open it."
  (let* ((index (project--index root))
         (state (car index))
         (found (length (nth 1 index))))
    (if (and (eq state 'complete) (= found 0))
        (message "No files in %s" root)
        (progn
          (setq project--find-root root)
          (cond ((eq state 'truncated)
                 (message (concat "%s files (truncated at %s -- raise "
                                  "find-file-recursive-limit)")
                          found find-file-recursive-limit))
                ((eq state 'walking)
                 (message "Still listing the files of %s: %s so far" root found)))
          (minibuffer-read (format "Find file in %s:" (project--abbreviate root))
                           (lambda (path)
                             (unless (string= path "")
                               (find-file-async (expand-file-name path root))))
                           'project-find-file--candidates
                           nil nil "project-find-file")))))

(defcommand project-find-file () nil
  "Open a file of the current project, choosing it by any part of its path
relative to the root."
  (let ((root (project--current)))
    (if root (project--find-file-in root))))

;; ---------------------------------------------------------------------------
;; Searching, building and running a command at the root
;; ---------------------------------------------------------------------------
;;
;; The question is asked first, and the answer handed to the command inside
;; `with-directory': a prompt answered after the call has returned would run
;; in the working directory.

(defun project--ask-and-run (root prompt key command)
  "Ask PROMPT about ROOT, and call COMMAND with the answer, at ROOT."
  (minibuffer-read (format "%s (%s):" prompt (project--abbreviate root))
                   (lambda (answer)
                     (unless (string= answer "")
                       (with-directory root
                                       (lambda () (funcall command answer)))))
                   nil nil nil key))

(defcommand project-grep () nil
  "Ask for a text and list every line containing it in the files under the
current project's root, as `grep' does."
  (let ((root (project--current)))
    (if root
        (project--ask-and-run root "Search project for" "project-grep" 'grep))))

(defcommand project-compile () nil
  "Ask for a command and run it at the current project's root, as `compile'
does: its output keeps the root, so g runs it there again."
  (let ((root (project--current)))
    (if root
        (project--ask-and-run root "Compile project" "project-compile"
                              'compile))))

(defcommand project-shell-command () nil
  "Ask for a shell command and run it at the current project's root, as
`shell-command' does."
  (let ((root (project--current)))
    (if root
        (project--ask-and-run root "Shell command in project"
                              "project-shell-command" 'shell-command))))

(defun project-compile-command (command)
  "Run COMMAND, a string, at the current project's root, as `project-compile'
runs what it asks for, and return t; nil, after saying why, outside a
project. What the menus of the languages run to build and test.

Example:
  (project-compile-command \"cargo test\")"
  (let ((root (project--current)))
    (if root
        (progn (with-directory root (lambda () (compile command)))
               t)
        nil)))

;; ---------------------------------------------------------------------------
;; Going to another project, declaring one, forgetting one
;; ---------------------------------------------------------------------------

(defun project--known-candidates (input)
  "The projects known that match INPUT, best first."
  (fuzzy-filter input (project-known-projects)))

(defcommand project-switch () nil
  "Ask for one of the projects known, or any directory, and find a file in
it as `project-find-file' does."
  (if (null (project-known-projects))
      (message "No projects known yet: a C-x p command in one makes it known")
      (minibuffer-read "Switch to project:"
                       (lambda (answer)
                         (unless (string= answer "")
                           (let ((root (project--directory answer)))
                             (if (file-directory-p root)
                                 (project--find-file-in (project--remember root))
                                 (message "%s is not a directory" root)))))
                       'project--known-candidates
                       nil nil "project-switch")))

(defcommand project-declare-root (directory) ("fDeclare project root: ")
  "Make DIRECTORY a project root whatever it holds, from now on and in the
next runs, and make it known."
  (let ((root (project--directory directory)))
    (if (not (file-directory-p root))
        (message "%s is not a directory" root)
        (let ((declared (session-value 'project-declared-roots)))
          (unless (member root declared)
            (set-session-value 'project-declared-roots (cons root declared)))
          (project--remember root)
          (message "Project root: %s" root)))))

(defcommand project-forget () nil
  "Ask for one of the projects known and forget it: it is no longer offered,
and no longer a root if it was declared with `project-declare-root'."
  (if (null (project-known-projects))
      (message "No projects known")
      (minibuffer-read "Forget project:"
                       (lambda (answer)
                         (unless (string= answer "")
                           (let ((root (project--directory answer)))
                             (project--drop root 'project-known)
                             (project--drop root 'project-declared-roots)
                             (message "Forgot %s" root))))
                       'project--known-candidates
                       nil nil "project-forget")))

;; ---------------------------------------------------------------------------
;; Keys
;; ---------------------------------------------------------------------------

(define-key nil "C-x p f" 'project-find-file)
(define-key nil "C-x p g" 'project-grep)
(define-key nil "C-x p c" 'project-compile)
(define-key nil "C-x p !" 'project-shell-command)
(define-key nil "C-x p p" 'project-switch)
(define-key nil "C-x p b" 'project-switch-to-buffer)
(define-key nil "C-x p e" 'project-eshell)
(define-key nil "C-x p d" 'project-dired)
(define-key nil "C-x p k" 'project-kill-buffers)

(log "project loaded")
(provide 'project)
