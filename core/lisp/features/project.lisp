;;; project --- the directory a file belongs to, and commands that work there.
;;;
;;; A project is a root directory, found above the current file by the names
;;; of `project-markers' or declared by hand; C-x p finds a file, searches,
;;; builds and runs a command there, and C-x p p goes to a project known from
;;; an earlier run. User manual: the `project' page.
;;; Design: `doc/architettura-editor.typ, <mod-project>`.

(require 'commands)
(require 'debug)
(require 'dired)
(require 'occur)
(require 'shell)
(require 'compile)

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
current buffer's file, or the working directory.

Example:
  (project-root \"~/src/app/src/main.rs\")"
  (let ((start (project--start directory)))
    (or (project--declared-above start)
        (project--marked-above start))))

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

(log "project loaded")
(provide 'project)
