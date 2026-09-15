;;; completion-at-point --- the sources `completion-at-point' asks.
;;;
;;; The command itself is in Rust, because it is the mechanism: it holds the
;;; list of sources, decides which text is being completed, merges what they
;;; offer and puts the answer in the buffer. What any particular source
;;; *knows* is not mechanism -- it is a question about files, or buffers, or
;;; the interpreter -- and that is what this file is.
;;;
;;; Each one can be removed on its own:
;;;
;;;   (set-completion-functions nil '(capf-file-name capf-symbols))
;;;
;;; # The one rule to follow when writing another
;;;
;;; Return `(START END CANDIDATES)', where START and END are the text you are
;;; offering to replace -- and get them from `bounds-of-thing-at-point' unless
;;; you are completing something that genuinely is not a symbol.
;;;
;;; `completion-at-point' merges the sources that claim **the same** START and
;;; END and passes over the ones that disagree, since two sources completing
;;; different spans of text cannot both be right. So a source that scanned
;;; backwards over its own idea of a word would not merely be eccentric: it
;;; would vanish from every list that another source answered first, and it
;;; would do so silently.
;;;
;;; `capf-file-name' is the one source here that *should* disagree, and does:
;;; a path claims the slashes and dots as well, so when point is inside one it
;;; wins the region outright and the symbol sources stand down.
;;;
;;; # Filtering is not your job
;;;
;;; Offer everything that could go there. `completion-at-point' narrows the
;;; list against what is already written, in one place, so that binding
;;; `*completion-filter-function*' makes every source fuzzy at once instead of
;;; making each of them separately wrong.

;; ---------------------------------------------------------------------------
;; Files
;; ---------------------------------------------------------------------------

(defun capf-file-name ()
  "Complete a path when point is inside something shaped like one.

`file-name-directory' is both the test and the answer: it returns nil for text
with no separator in it, which is exactly \"this is not a path\", and it is the
only test that is right on every platform -- a module comparing against \"/\"
would be writing down one operating system's answer and calling it the rule."
  (let ((bounds (bounds-of-thing-at-point 'filename)))
    (if bounds
        (let* ((start (nth 0 bounds))
               (end (nth 1 bounds))
               (text (buffer-substring start end))
               (directory (file-name-directory text)))
          (if directory
              (list start end
                    (mapcar (lambda (entry) (concat directory entry))
                            (list-dir directory))))))))

;; ---------------------------------------------------------------------------
;; The language of the buffer
;; ---------------------------------------------------------------------------

(defun capf-mode-keywords ()
  "Complete the words this buffer's language has of its own.

Empty for a mode that never called `set-mode-keywords', which costs nothing:
a source with no candidates contributes none rather than claiming the region
and then having nothing to say."
  (capf--offer (mapcar (lambda (word) (cons word "keyword"))
                       (mode-keywords))))

;; ---------------------------------------------------------------------------
;; The interpreter
;; ---------------------------------------------------------------------------

(defun capf-symbols ()
  "Complete the names the interpreter can actually resolve.

Asked for on every keystroke rather than remembered, which is the whole value
of it: define a function and it completes, misspell one and it does not. The
three namespaces are kept apart so the answer says which kind of thing it
found -- a macro is not called the way a function is."
  (capf--offer
   (append (mapcar (lambda (name) (cons name "function")) (all-functions))
           (append (mapcar (lambda (name) (cons name "macro")) (all-macros))
                   (mapcar (lambda (name) (cons name "variable")) (all-variables))))))

;; ---------------------------------------------------------------------------
;; Buffers
;; ---------------------------------------------------------------------------

(defun capf-buffer-names ()
  "Complete the name of a live buffer."
  (capf--offer (mapcar (lambda (name) (cons name "buffer"))
                       (all-buffer-names))))

(defun capf-buffer-words ()
  "Complete a word that is already written somewhere.

Last in the default list, and deliberately: it can offer something for almost
any prefix, so anything that knows more than \"this text exists\" should have
had its say first. Merging rather than racing is what keeps it from burying
them -- it adds to the list the others started instead of replacing it.

The word being typed is thrown away, and that is not tidiness. This source
reads the buffer, so the half-written word at point is in it, and offering it
back means the candidates have it as their common prefix -- so with no
presenter loaded, pressing the key fills in exactly what was already there and
appears to do nothing at all."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if bounds
        (let* ((start (nth 0 bounds))
               (end (nth 1 bounds))
               (typed (buffer-substring start end))
               (candidates (capf--words-except typed)))
          (if candidates (list start end candidates))))))

(defun capf--words-except (typed)
  "Every word in every buffer except TYPED, labelled with where it came from.

The current buffer is read first so a word in two places is offered by the
nearer one -- `completion-at-point' keeps the first of any repeated value, so
the description that survives is the one from the buffer being edited.

Built by consing and reversed at the end rather than appended to in the loop:
appending copies the whole list each time round, and this runs over every word
in every buffer, on a key press."
  (let ((here (current-buffer))
        (found nil))
    (dolist (word (buffer-words))
      (if (not (string= word typed))
          (setq found (cons (cons word "word") found))))
    (dolist (name (all-buffer-names))
      (if (not (string= name here))
          (dolist (word (buffer-words name))
            (if (not (string= word typed))
                (setq found (cons (cons word name) found))))))
    (reverse found)))

;; ---------------------------------------------------------------------------
;; The shape every source above shares
;; ---------------------------------------------------------------------------

(defun capf--offer (candidates)
  "Offer CANDIDATES for the symbol at point, or nothing if there is no symbol.

Every source that completes a *name* ends in this, so that they all claim the
same region and therefore all merge. A source that wrote out the same three
lines itself would work until someone changed one of the copies."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (and bounds candidates)
        (list (nth 0 bounds) (nth 1 bounds) candidates))))

;; ---------------------------------------------------------------------------
;; The default list
;; ---------------------------------------------------------------------------
;;
;; Order is not cosmetic. The first source to answer fixes the region, and
;; where two offer the same name the first one's description survives -- so
;; this reads most-specific first, with the source that will answer to anything
;; at the end.
;;
;; A mode's own sources are tried before all of these, without having to be
;; inserted here: see `add-completion-function'.

(set-completion-functions nil
                          '(capf-file-name
                            capf-mode-keywords
                            capf-symbols
                            capf-buffer-names
                            capf-buffer-words))

(log "End of the completion-at-point.lisp")
