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
  (let ((name (file-name-nondirectory file))
        (cut nil)
        (n 0))
    (while (< n (length name))
      (if (and (null cut) (string= "." (substring name n (+ n 1))))
          (setq cut n))
      (setq n (+ n 1)))
    (if cut (substring name 0 cut) name)))

(defun manpage-candidates (input)
  "Manual page names matching INPUT, best first.

Walks `manpage-path' and this editor's own pages. Used both for the `manpage'
prompt and as a completion source, which is why it takes a pattern rather than
returning everything: the list on a full system is tens of thousands of names."
  (fuzzy-filter input (manpage-names)))

(defun manpage-names ()
  "Every manual page name that can be found, rsedit's first.

Walked each time rather than cached. The result is only ever handed to
`fuzzy-filter', and a cache would be one more thing to invalidate when
`manpage-path' changes."
  ;; `mapcar' per directory, and one `append' per directory to join them.
  ;;
  ;; What this used to do was `(setq names (append names (list one-name)))'
  ;; per file, which copies everything gathered so far on every step: building
  ;; an n-element list that way costs about n squared units of work. The
  ;; interpreter charges for the walk, so on a system with a real set of manual
  ;; pages -- five to thirty thousand of them -- this did not merely go slowly,
  ;; it ran out of fuel and gave up. The budget gives out somewhere around
  ;; 4,500 elements.
  ;;
  ;; `mapcar' builds its result once, and the joins are per *directory*, of
  ;; which there is a handful.
  (let ((names nil))
    (mapc (lambda (directory)
            (setq names
                  (append names
                          (mapcar 'manpage--strip-extensions
                                  (nth 1 (directory-files-recursive directory 20000))))))
          (append (let ((ours (manpage--rsedit-directory)))
                    (if (and ours (file-directory-p ours)) (list ours) nil))
                  manpage-path))
    names))

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

(log "manpage loaded")
