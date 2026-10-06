;;; manpage --- reading manual pages, the system's and this editor's.
;;;
;;; `K' on a word opens its page. `M-x manpage' asks for one, completing over
;;; everything installed. rsedit's own pages are searched first, so `K' on
;;; `insert' while writing Lisp gives you this editor's `insert' rather than
;;; nothing at all.
;;;
;;; # Why `man' does the rendering
;;;
;;; Installed pages are gzipped troff. Rendering them means a gzip decompressor
;;; and a troff subset, which is a project rather than a module -- and `man' is
;;; already on any machine that has pages to read. So the module's job is to
;;; decide *which* page, and `man' formats it.
;;;
;;; `manpage-path' is handed to it as MANPATH, so the variable controls what is
;;; found as well as what is listed. A path list that only affected the
;;; completion candidates would be the kind of half-truth that wastes an
;;; afternoon.
;;;
;;; # How the formatting survives
;;;
;;; `man' marks bold and underline with overstrike -- `e\be' -- which is
;;; unreadable if left in. `parse-overstrike' takes it apart into plain text and
;;; the spans that were emphasised, and each span becomes an overlay.
;;;
;;; Overlays are what make this possible at all. A mode's colouring is a set of
;;; patterns over the text, and "this word, because `man' doubled its letters"
;;; is not a pattern -- there is nothing about the word itself to match on. The
;;; syntax rules at the bottom still do the structural part, which patterns are
;;; good at: a heading is a heading because it is a word alone at the left
;;; margin. The two together give a page that reads the way it does in a
;;; terminal, emphasis mid-sentence included.

(make-mode 'manpage-mode)

(defconst manpage-buffer-name "*Manual*"
  "The one buffer pages are shown in. Reading a second page replaces the first,
the way it does in `man' itself -- a page is something you consult, not
something you accumulate.")

;; Where pages are looked for, and what is offered for completion. Given to
;; `man' as MANPATH, so this is the whole answer to "which pages exist".
;;
;; The defaults are the usual Unix locations. On a system that keeps them
;; somewhere else, or for a toolchain that installs its own, add to this.
(setq manpage-path
      (if (getenv "MANPATH")
          (split-string (getenv "MANPATH") (path-separator))
          '("/usr/share/man"
            "/usr/local/share/man"
            "/usr/local/man"
            "/opt/local/share/man")))

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
  "The text of rsedit's page for NAME, or nil if it has none.

Searched before the system's, which is the right way round while writing rsedit
Lisp: `insert' and `point' mean this editor's, and a name that is only a Unix
command falls through to `man' unchanged."
  (let ((directory (manpage--rsedit-directory)))
    (if (null directory)
        nil
        (read-file-to-string
         (concat (file-name-as-directory directory) name ".txt")))))

(defun manpage--system-page (name)
  "The text of the system's page for NAME, or nil if `man' found none.

MANPATH is set from `manpage-path' first, so what is rendered is what this
module said should be searched."
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
  "Whether LOCATED is `man -w' saying it found a page.

`man -w' documents itself as printing the *location* of the page it would
format, so an answer that is a path means yes and anything else means no.

Checking the shape rather than the exit status, because the status cannot be
trusted: a system with its manuals stripped out -- a minimal container, most
build images -- ships a stub `man' that prints an explanatory paragraph to
standard output and exits 0 whatever it is asked. Taking that at its word gives
a manual page whose entire content is an apology for not having manual pages."
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
  "Where a page's own emphasis sits among overlaps.

Above nothing in particular today -- it is the only thing making overlays in a
manual buffer. Named rather than written as a bare 10 so that whatever is added
next has something to be higher or lower than.")

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
  "The face KIND -- `bold' or `underline' -- should be drawn in.

Two faces of this module's own rather than reusing `keyword' and so on: a
manual page's bold means "this is emphasised", not "this is a keyword", and a
theme should be able to say so."
  (if (eq kind 'underline) 'manpage-underline 'manpage-bold))

(defcommand manpage (name) ("sManual page: ")
  "Show the manual page for NAME. Bound to M-x manpage.

rsedit's own pages are searched before the system's, so a name that is both --
`insert', `sort' -- gives you this editor's."
  (let ((text (manpage--text name)))
    (if text
        (manpage--show name text)
        (message "No manual entry for %s" name))))

(defcommand manpage-at-point () nil
  "Show the manual page for the word under the cursor. Bound to K.

The thing you actually want while reading code: point at a name, press one key,
read what it does."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (null bounds)
        (message "No word here")
        (manpage (buffer-substring (nth 0 bounds) (nth 1 bounds))))))

;; ---------------------------------------------------------------------------
;; What pages there are
;; ---------------------------------------------------------------------------

(defun manpage--strip-extensions (file)
  "FILE without its section and compression suffixes: \"ls.1.gz\" => \"ls\".

Everything from the first dot, because a page is named for a command and a
command does not have one."
  ;; One `string-match', not a loop over the characters.
  ;;
  ;; What this used to do was walk the name a character at a time asking
  ;; `(< n (length name))' -- and `length' on a string charges per character,
  ;; so each step cost the length of the name and the whole thing cost its
  ;; square. Measured: 184 units for a 7-character name, 808 for a 20-character
  ;; one. Called once per page, on a system with twenty thousand of them, that
  ;; is ten million units spent deciding what the files are called -- the whole
  ;; fuel budget, before anything is matched against anything.
  (let ((name (file-name-nondirectory file)))
    (if (string-match "^[^.]+" name)
        (match-string 0 name)
        name)))

(defun manpage-candidates (input)
  "Manual page names matching INPUT, best first.

Walks `manpage-path' and this editor's own pages. Used both for the `manpage'
prompt and as a completion source, which is why it takes a pattern rather than
returning everything: the list on a full system is tens of thousands of names."
  (fuzzy-filter input (manpage-names)))

(setq manpage--names nil)
(setq manpage--names-for nil)
(setq manpage--building-for nil)
(setq manpage--index-announce nil)

(defconst manpage-index-job 'manpage-index
  "The name the background index build runs under.

Named rather than written out three times, and named at all because that is
how a job is stopped and replaced: `(stop-worker manpage-index-job)' stops the
build, and starting a second one under the same name retires the first instead
of running two walks of the same directories against each other.")

(defun manpage-names ()
  "Every manual page name that can be found, rsedit's first.

Answered from `manpage--names', which `manpage--reindex' fills in the
background.

# Why this is cached now, having deliberately not been

It used to walk on every call, on the argument that a cache is one more thing
to invalidate. That argument was right about the cost and wrong about the
scale: the walk itself is milliseconds, but the *names* are stripped of their
extensions one `string-match' at a time, and a completion source is asked
again on every keystroke and every Tab. On a system with four thousand pages
that was three seconds per press, on the thread that draws.

# What it answers while the list is still being built

What there is so far. Every name in a half-built list is a real page -- the
build appends whole directories -- so a partial answer is a short one rather
than a wrong one, and it arrives now. The alternative is walking the tree
here, which is the three-second pause this exists to remove, and doing it
while a background job is part-way through doing the same thing.

What is invalidated, and when, is the whole of the cost: the list is rebuilt
when `manpage-path' is not the one it was built from. Nothing else can change
the answer except a page appearing on disk, and `manpage-reindex' is there for
that."
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
  "The page names in DIRECTORY and below it.

`mapcar' rather than appending one name at a time: building an n-element list
by `(setq names (append names (list one)))' copies everything gathered so far
on every step, which is n squared units of work, and the interpreter charges
for the walk. On a system with a real set of manual pages -- five to thirty
thousand of them -- that did not merely go slowly, it ran out of fuel and gave
up somewhere around 4,500 names."
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
  "A fiber that fills `manpage--names' for FOR-PATH, a directory at a time.

One directory per `(yield)', which is what makes the list grow visibly rather
than appearing whole at the end: a yield is a progress point, and the progress
callback is what tells a prompt that has the list open that there is more of
it.

FOR-PATH is carried rather than read from `manpage-path' each time round,
because they can stop being the same thing: the user may set the path while
this is running. A job never assumes the editor stood still, so this checks
before it publishes anything and gives up if the answer it is building is no
longer the answer to any question."
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
             (yield)))))))

(defun manpage--index-progress (name)
  "Another directory's worth of page names is in the list.

NAME is the job, which is not used: there is one of these and it is this one.
The argument is there because `background-call' hands its callbacks the name
they ran under rather than what the job produced -- what it produced is in the
variable, and the variable is what anybody reading it would read anyway.

All this does is say that the candidates changed. What that means to whatever
is showing them is not this module's business, and `completion-invalidate'
reaches a completion strip without this module having to know one exists."
  (completion-invalidate))

(defun manpage--index-done (name finished)
  "The build under NAME has ended, FINISHED saying whether it reached the end.

A stopped build -- one that raised, or spent its allowance -- leaves the names
it had gathered in place and does *not* mark them complete, so the next thing
to ask gets a fresh walk rather than a list that is short for ever.

The mode keeps itself consistent with the new state, which here is three
things: the list is complete or it is not, nothing is building any more, and
whatever is showing candidates is told."
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
  "Start building the list of page names in the background. Returns the name.

Called when the module loads, so that the first `C-h m' has an answer waiting
rather than paying for the walk itself.

Starting it again while it is running replaces it, which is what should happen
when `manpage-path' has changed: the walk in progress is of the old
directories."
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
  "Build the list here and now, and return it.

For the caller that cannot wait -- `manpage-names' asked before anything has
been built. Whatever was being built in the background is stopped first: it is
walking the same directories to reach the same answer, and letting it finish
would have it publish a list for the path it started with over the one built
here."
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
  "Look for manual pages again, in the background.

For after installing something. The list is otherwise rebuilt only when
`manpage-path' changes, since nothing else the editor can see makes it wrong."
  (setq manpage--index-announce t)
  (manpage--reindex)
  (message "Looking for manual pages..."))

(defcommand manpage-prompt () nil
  "Ask which manual page to open, completing over the ones installed."
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
