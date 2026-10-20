;;; manpage --- reading manual pages, this editor's and the system's.
;;;
;;; C-h m asks for a page, completing over every one installed, and K opens
;;; the page named by the word under the cursor; this editor's pages come
;;; first. User manual: the `getting-help' page.
;;;
;;; `man' formats the system's pages, with `manpage-path' as MANPATH, so the
;;; variable decides what is found as well as what is listed. The bold and the
;;; underlining it marks by overstriking become overlays, and the headings are
;;; coloured from the layout. The list of names is built in the background.

(make-mode 'manpage-mode)

(defconst manpage-buffer-name "*Manual*"
  "The one buffer pages are shown in: a second page takes the place of the
first.")

(defvar manpage-path
  (if (getenv "MANPATH")
      (split-string (getenv "MANPATH") (path-separator))
      '("/usr/share/man"
        "/usr/local/share/man"
        "/usr/local/man"
        "/opt/local/share/man"))
  "The directories the system's manual pages are looked for in, given to man
as MANPATH. It starts as the MANPATH the editor was started with, or the usual
places when there is none.")

;; ---------------------------------------------------------------------------
;; Finding a page
;; ---------------------------------------------------------------------------

(defun manpage--join (parts separator)
  "PARTS joined with SEPARATOR."
  (let ((out ""))
    (mapc (lambda (part)
            (setq out (if (string= out "") part (concat out separator part))))
          parts)
    out))

(defun manpage--rsedit-directory ()
  "Where this editor's own pages were installed, or nil."
  (data-directory "man"))

(defun manpage--rsedit-page (name)
  "The text of this editor's page NAME, or nil."
  (let ((directory (manpage--rsedit-directory)))
    (if (null directory)
        nil
        (read-file-to-string
         (concat (file-name-as-directory directory) name ".txt")))))

(defun manpage--system-page (name)
  "The text of the system's page NAME, formatted by man, or nil."
  (setenv "MANPATH" (manpage--join manpage-path (path-separator)))
  ;; Asked whether the page exists *before* asking for it. `man -w' prints the
  ;; file it would format and nothing at all when there is none, which is the
  ;; documented way to ask -- whereas `man' itself writes "No manual entry for
  ;; x" to standard output on some systems, so an absent page comes back
  ;; looking exactly like a found one.
  (let ((located (shell-command-to-string
                  (format "man -w %s 2>/dev/null" name))))
    (if (not (manpage--located-p located))
        nil
        ;; `-P cat' stops `man' starting a pager of its own, which would sit
        ;; there waiting for a keypress that is never coming. The output still
        ;; carries overstrike, which is stripped when it is shown.
        (let ((text (shell-command-to-string
                     (format "man -P cat %s 2>/dev/null" name))))
          (if (or (null text) (string= text "")) nil text)))))

(defun manpage--located-p (located)
  "Whether LOCATED, what man -w printed, is the path of a page."
  ;; The shape, not the exit status: a system with its manuals stripped out
  ;; ships a man that prints an apology and exits 0 whatever it is asked.
  (and located
       (let ((trimmed (manpage--trim located)))
         (and (not (string= trimmed ""))
              (string= "/" (substring trimmed 0 1))))))

(defun manpage--trim (text)
  "TEXT without leading or trailing whitespace."
  (let ((start 0)
        (end (length text)))
    (while (and (< start end)
                (member (substring text start (+ start 1)) '(" " "\t" "\n" "\r")))
      (setq start (+ start 1)))
    (while (and (< start end)
                (member (substring text (- end 1) end) '(" " "\t" "\n" "\r")))
      (setq end (- end 1)))
    (substring text start end)))

(defun manpage--text (name)
  "The page for NAME from wherever it is, or nil."
  (let ((ours (manpage--rsedit-page name)))
    (if ours ours (manpage--system-page name))))

;; ---------------------------------------------------------------------------
;; Showing it
;; ---------------------------------------------------------------------------

(defconst manpage-emphasis-priority 10
  "The priority of the overlays a page's bold and underlining are drawn with.")

(defun manpage--show (name text)
  "Put TEXT in the manual buffer as the page for NAME, emphasis and all."
  (buffer-create manpage-buffer-name 'manpage-mode)
  (switch-to-buffer manpage-buffer-name)
  (let* ((parsed (parse-overstrike text))
         (plain (car parsed))
         (spans (nth 1 parsed)))
    (set-buffer-read-only nil)
    (clear-buffer)
    (insert plain)
    (set-buffer-read-only t)
    ;; The old overlays first: a buffer reused for a second page would
    ;; otherwise keep the first one's emphasis at the first one's offsets,
    ;; which is emphasis scattered over unrelated words.
    (remove-overlays 'manpage)
    (mapc (lambda (span)
            (make-overlay (nth 0 span)
                          (nth 1 span)
                          (manpage--face-for (nth 2 span))
                          manpage-emphasis-priority
                          'manpage))
          spans))
  (goto-char 0)
  (message "Manual page %s" name))

(defun manpage--face-for (kind)
  "The face the emphasis KIND, `bold' or `underline', is drawn in."
  (if (eq kind 'underline) 'manpage-underline 'manpage-bold))

(defcommand manpage (name) ("sManual page: ")
  "Show the manual page NAME: this editor's when it has one, else the
system's."
  (let ((text (manpage--text name)))
    (if text
        (manpage--show name text)
        (message "No manual entry for %s" name))))

(defcommand manpage-at-point () nil
  "Show the manual page named by the word under the cursor. K in a manual
page."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (null bounds)
        (message "No word here")
        (manpage (buffer-substring (nth 0 bounds) (nth 1 bounds))))))

;; ---------------------------------------------------------------------------
;; What pages there are
;; ---------------------------------------------------------------------------

(defun manpage--strip-extensions (file)
  "FILE without its section and compression suffixes: \"ls.1.gz\" => \"ls\"."
  ;; One `string-match': `length' on a string charges per character, so a
  ;; loop asking it was quadratic, and ran out of fuel on a full system.
  (let ((name (file-name-nondirectory file)))
    (if (string-match "^[^.]+" name)
        (match-string 0 name)
        name)))

(defun manpage-candidates (input)
  "Manual page names matching INPUT, best first."
  (fuzzy-filter input (manpage-names)))

(setq manpage--names nil)
(setq manpage--names-for nil)
(setq manpage--building-for nil)
(setq manpage--index-announce nil)

(defconst manpage-index-job 'manpage-index
  "The name the background build of the list of pages runs under, for
`stop-worker'.")

(defun manpage-names ()
  "Every manual page name found, this editor's first: the list the background
build fills in, or, when nothing has been built for `manpage-path', one built
now."
  ;; Kept, not walked on every call: a completion source is asked on every Tab,
  ;; and stripping thousands of names took seconds. While a build runs, what
  ;; it has found so far is a short answer but not a wrong one, since it adds
  ;; whole directories; the list is built again when `manpage-path' changes.
  (cond
   ;; Being built for the path in force now, and there is something to hand
   ;; out: the previous complete list until the first directory of the new
   ;; walk lands, and the new one growing after that. Asked before
   ;; `manpage--names-for', which a rebuild of the *same* path leaves looking
   ;; true while the list underneath it is half replaced.
   ;;
   ;; `manpage--names' being non-empty is half the condition and not a
   ;; formality. The first moments of a session are exactly when a build is
   ;; running and has produced nothing, and an empty list is not a short
   ;; answer -- it is a wrong one, and it is what `C-h m' would get for
   ;; pressing Tab promptly enough. The walk below is what that press is
   ;; worth waiting for; it is also what stops the build, since the two are
   ;; walking the same directories for the same answer.
   ((and manpage--names (equal manpage--building-for manpage-path)) manpage--names)
   ;; Built, and for the path in force now.
   ((equal manpage--names-for manpage-path) manpage--names)
   ;; Never asked before, or the path has changed since and nothing is
   ;; building for the new one. Walked here, because the caller wants an
   ;; answer now and an empty list is a wrong one.
   (t (manpage--reindex-now))))

(defun manpage--directories ()
  "Where to look for pages, this editor's own first."
  (append (let ((ours (manpage--rsedit-directory)))
            (if (and ours (file-directory-p ours)) (list ours) nil))
          manpage-path))

(defun manpage--names-in (directory)
  "The page names in DIRECTORY and below it."
  ;; `mapcar', not appending one at a time, which is quadratic and ran out of
  ;; fuel on a system with thousands of pages.
  (mapcar 'manpage--strip-extensions
          (nth 1 (directory-files-recursive directory 20000))))

;; ---------------------------------------------------------------------------
;; Building the list in the background
;; ---------------------------------------------------------------------------
;;
;; The shape `background-call' asks for: a job that computes and stores, and
;; two callbacks that run where a command runs and are where anything is told
;; anything. See the `background' manual page for the rule.

(defun manpage--index-fiber (for-path)
  "A fiber that fills `manpage--names' for FOR-PATH, a directory at a time."
  ;; One directory per yield, so that the list grows where it can be seen, and
  ;; `continue', since there is nothing to wait for. FOR-PATH is carried: if
  ;; `manpage-path' changes meanwhile, the build gives up.
  (fiber
   (let ((gathered nil)
         (left (manpage--directories)))
     (while left
       (if (not (equal for-path manpage-path))
           ;; The path changed underneath. What has been gathered is for a
           ;; question nobody is asking; the job that was started for the new
           ;; path will answer the one they are.
           (setq left nil)
           (progn
             (setq gathered (append gathered (manpage--names-in (car left))))
             (setq left (cdr left))
             ;; Published as it grows, not at the end.
             (setq manpage--names gathered)
             (yield 'continue)))))))

(defun manpage--index-progress (name)
  "Say that the list of pages has grown; NAME, the job's, is not used."
  (completion-invalidate))

(defun manpage--index-done (name finished)
  "Note that the build under NAME has ended, FINISHED saying whether it reached
the end."
  ;; A stopped build leaves its names but not marked complete, so that the
  ;; next question walks again.
  (if (and finished (equal manpage--building-for manpage-path))
      (setq manpage--names-for manpage--building-for))
  (setq manpage--building-for nil)
  (completion-invalidate)
  (if manpage--index-announce
      (progn
        (setq manpage--index-announce nil)
        (message (if finished
                     (format "%d manual pages" (length manpage--names))
                     "Looking for manual pages did not finish -- see the log")))))

(defun manpage--reindex ()
  "Start building the list of page names in the background, and return the
job's name. A build already running is replaced."
  ;; `manpage--names' is left alone. It is the previous answer, which is a
  ;; better thing to hand out for the moment before the first directory lands
  ;; than an empty list would be -- and `manpage--names-for' going to nil is
  ;; what stops it being called complete in the meantime.
  (setq manpage--names-for nil)
  (setq manpage--building-for manpage-path)
  (background-call manpage-index-job
                   (manpage--index-fiber manpage-path)
                   'manpage--index-progress
                   'manpage--index-done))

(defun manpage--reindex-now ()
  "Build the list of page names here and now, and return it, stopping a build
in the background first."
  (stop-worker manpage-index-job)
  (setq manpage--building-for nil)
  (setq manpage--names (let ((names nil))
                         (mapc (lambda (directory)
                                 (setq names (append names (manpage--names-in directory))))
                               (manpage--directories))
                         names))
  (setq manpage--names-for manpage-path)
  manpage--names)

(defcommand manpage-reindex () nil
  "Look for manual pages again, in the background, and say how many there are
when done: for after installing some."
  (setq manpage--index-announce t)
  (setq manpage--homes nil)
  (manpage--reindex)
  (message "Looking for manual pages..."))

(defcommand manpage-prompt () nil
  "Ask for a manual page, completing over every one installed, and show it.
C-h m."
  (minibuffer-read "Manual page:"
                   (lambda (name) (if (string= name "") nil (manpage name)))
                   'manpage-candidates
                   nil))

;; A completion source, so `C-M-i' in a manual buffer offers page names -- and
;; so anything else that wants them has a function to call.
(add-completion-function 'manpage-mode 'capf-manpage-names)

(defun capf-manpage-names ()
  "Complete the symbol at point over manual page names."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (null bounds)
        nil
        (list (nth 0 bounds) (nth 1 bounds) (manpage-names)))))

;; ---------------------------------------------------------------------------
;; Which page is a name's home
;; ---------------------------------------------------------------------------
;;
;; A page lists in its SYNOPSIS the names it documents, one per line:
;; `(name ARGS)' for a function or a command, `name' alone for a variable.
;; That page is the name's home, and `C-h f' and `C-h v' point to it.

(defun manpage--synopsis-names (text)
  "The names the SYNOPSIS of the page TEXT lists, in order; nil without one."
  (if (not (string-match "\nSYNOPSIS\n" text))
      nil
      (let* ((start (match-end 0))
             (end (if (string-match "\n[^ \n]" text start)
                      (match-beginning 0)
                      (length text)))
             (names nil))
        (mapc (lambda (line)
                (if (string-match "[^ ()]+" line)
                    (setq names (cons (match-string 0 line) names))))
              (split-string (substring text start end) "\n"))
        (reverse names))))

(setq manpage--homes nil)

(defun manpage--read-homes ()
  "An alist from each name an rsedit page's SYNOPSIS lists to that page."
  (let ((directory (manpage--rsedit-directory))
        (homes nil))
    (if (and directory (file-directory-p directory))
        (mapc (lambda (file)
                (if (string-match "^(.+)\\.txt$" file)
                    (let ((page (match-string 1 file))
                          (text (read-file-to-string
                                 (concat (file-name-as-directory directory) file))))
                      (mapc (lambda (name) (setq homes (cons (cons name page) homes)))
                            (manpage--synopsis-names text)))))
              (list-dir directory)))
    homes))

(defun manpage-home (name)
  "The page of this editor's manual whose SYNOPSIS lists NAME, a symbol or a
string, or nil: the page C-h f and C-h v name."
  (if (null manpage--homes)
      (setq manpage--homes (manpage--read-homes)))
  (cdr (assoc (if (symbolp name) (symbol-name name) name) manpage--homes)))

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(define-key nil "C-h m" 'manpage-prompt)
(define-key 'manpage-mode "q" '(close-buffer manpage-buffer-name))
(define-key 'manpage-mode "n" 'next-line)
(define-key 'manpage-mode "p" 'previous-line)
(define-key 'manpage-mode "K" 'manpage-at-point)

;; The emphasis, recovered from the structure rather than from the overstrike
;; that was stripped -- see the module header.

;; A section heading is a word alone at the left margin, in capitals.
(add-syntax-rule 'manpage-mode "^[A-Z][A-Z0-9 ]*$" 'keyword)
;; An option, indented and starting with a dash.
(add-syntax-rule 'manpage-mode "^[ \t]+(--?[a-zA-Z0-9][a-zA-Z0-9-]*)" 'function 1)
;; The name being documented, as `man' writes it in the header line.
(add-syntax-rule 'manpage-mode "^([A-Za-z0-9_.-]+)\\([0-9][a-zA-Z]*\\)" 'type 1)

;; The two faces the page's own emphasis is drawn in. Given an appearance here
;; because this module invents them -- the editor has never heard of either
;; until these lines name them.
(set-face 'manpage-bold nil nil '("bold"))
(set-face 'manpage-underline nil nil '("underline"))

;; Started now, in the background, so the first `C-h m' has a list waiting
;; rather than walking the tree while somebody is holding Tab down. Nothing
;; waits for it: while it runs `manpage-names' answers with what has been
;; found so far, and if nothing has it walks the tree itself. That is what
;; makes this an optimisation rather than a race.
;;
;; Re-loading this file starts it again rather than a second time, because the
;; job has a name -- see `background-call'.
(manpage--reindex)

(log "manpage loaded")
