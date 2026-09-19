;;; electric-pair --- closing delimiters that type themselves.
;;;
;;; Typing `(' gives you `()' with point between them; typing `)' when one is
;;; already there steps over it rather than adding a second; backspacing the
;;; `(' of an empty `()' takes the `)' with it.
;;;
;;; The pairs are not listed here. They come from the mode's own syntax table --
;;; the same one `forward-sexp' and the indenter read -- so a mode that called
;;; `set-syntax-pairs' has already said everything this needs to know, and a
;;; mode with unusual brackets gets pairing that matches them without doing
;;; anything further. A second list would be a second thing to keep in step.
;;;
;;; All of this hangs off `post-self-insert-hook', which runs after the command
;;; that typed a character and only after that command. A paste does not go
;;; through it, which is the point: pasted code already has its brackets, and
;;; pairing them again would double every one.

(setq electric-pair-mode t)

(setq electric-pair-skip-self t
      ;; Typing a closing delimiter where one already sits moves over it
      ;; instead of inserting another. Without this, every auto-inserted closer
      ;; has to be stepped over with an arrow key or deleted, and the feature
      ;; costs more keystrokes than it saves. Separated out because it is also
      ;; the part people most often want off: it makes the closing delimiter
      ;; you actually typed disappear, which feels wrong until it doesn't.
      )

;; ---------------------------------------------------------------------------
;; Looking around point
;; ---------------------------------------------------------------------------

(defun electric-pair--char-after ()
  "The character after point as a one-character string, or nil at end of buffer."
  (if (< (point) (point-max))
      (buffer-substring (point) (+ (point) 1))
      nil))

(defun electric-pair--char-before ()
  "The character before point as a one-character string, or nil at start of buffer."
  (if (> (point) (point-min))
      (buffer-substring (- (point) 1) (point))
      nil))

(defun electric-pair--in-string-or-comment (position)
  "Whether POSITION is inside a string or a comment.

Asked about a position rather than about point, and the difference matters: the
character that was just typed may be the one that *opened* the string, and
asking about point would report the string it just began as a reason not to
pair it."
  (let ((state (syntax-ppss position)))
    (or (nth 2 state) (nth 3 state))))

(defun electric-pair--balanced-p ()
  "Whether every list in the buffer is closed.

Asked of the *whole buffer*, and that is the part worth understanding.

The obvious question is \"is the list point is in already closed?\", and it gives
the wrong answer. A scanner matches a closer to the *innermost* opener, so just
after typing `{' inside a block that is already closed, the new brace appears to
have taken the outer one's `}' -- and the check refuses to pair in exactly the
case where pairing is wanted. It is not the scanner being wrong; innermost-first
is what matching means.

Depth at the end of the buffer has no such confusion. It counts openers that
nothing closes, wherever they are, and that is the question: a buffer with an
unclosed opener wants a closer, and a balanced one does not.

Costs a scan of the buffer. `post-self-insert-hook' already asks `syntax-ppss'
about the position before point on every character typed, so this roughly
doubles a cost that was already being paid rather than introducing one."
  (= 0 (nth 0 (syntax-ppss (point-max)))))

(defun electric-pair--word-after-p ()
  "Whether the character after point belongs to a word.

Pairing before a word is nearly always wrong: typing `(' in front of `foo'
means you are about to wrap it, and `()foo' is not that. Emacs makes this
configurable; here it is simply the rule, because no one has yet wanted the
other behaviour."
  (let ((next (electric-pair--char-after)))
    (and next (eq (syntax-class next) 'symbol))))

;; ---------------------------------------------------------------------------
;; The hook
;; ---------------------------------------------------------------------------

(defun electric-pair-post-self-insert ()
  "Pair, or skip over, the character that was just typed.

Run from `post-self-insert-hook', so point is immediately after the character
in question and the character is already in the buffer. That is why a closing
delimiter is inserted and point moved back, rather than anything being
rearranged: the opening one is where it should be already.

The order of the tests below is load-bearing. Quotes are decided *before* the
\"not inside a string\" rule rather than after it, because for a quote that rule
would refuse every closing quote there is: the state just before a closing
quote is, necessarily, inside the string it closes. A single test for \"in a
string\" ahead of the dispatch reads better and is wrong."
  (if electric-pair-mode
      (let ((typed (electric-pair--char-before)))
        (if typed
            ;; The state *before* the typed character, not at point. At point
            ;; it would describe the string or comment that this very
            ;; character may have just opened.
            (let* ((state (syntax-ppss (- (point) 1)))
                   (in-string (nth 2 state))
                   (in-comment (nth 3 state))
                   (class (syntax-class typed))
                   (partner (matching-delimiter typed))
                   (next (electric-pair--char-after)))
              (cond
               ;; A comment is prose. Nothing in it pairs -- not brackets, not
               ;; quotes, not apostrophes in particular.
               (in-comment nil)

               ;; Quotes, before the in-string test and for the reason given
               ;; above. Which of open and close this is cannot be read off the
               ;; character -- the same one does both -- so it is read off what
               ;; is next to it.
               ((eq class 'string)
                (cond
                 ((electric-pair--quotes-inhibited) nil)
                 ;; Sitting on the partner: this quote closes, so step over the
                 ;; one already there rather than adding a third.
                 ((and next (string= next typed) electric-pair-skip-self)
                  (progn (delete-backward-char) (forward-char)))
                 ;; Inside a string but not at its end -- an escaped quote, or
                 ;; one being added mid-text. Not an opening quote, so nothing
                 ;; to open.
                 (in-string nil)
                 ((electric-pair--word-after-p) nil)
                 (t (progn (insert typed) (backward-char)))))

               ;; Everything else leaves the contents of strings alone.
               (in-string nil)

               ;; An opener: put its partner after point and stay between
               ;; them -- unless the list it opens is closed already.
               ;;
               ;; That last part is what stops this making a mess of repairing
               ;; a brace. Delete the `{' from `fn a() {' and type it back,
               ;; and the `}' three lines below is still there: adding another
               ;; gives `{}' with a stray `}' after it, which the delete rules
               ;; then treat as an empty pair and take out together.
               ((and (eq class 'open) partner)
                (if (or (electric-pair--word-after-p)
                        (electric-pair--balanced-p))
                    nil
                    (progn (insert partner) (backward-char))))

               ;; A closer, where one is already doing the job.
               ((and (eq class 'close) electric-pair-skip-self)
                (electric-pair--closing typed next))

               ;; A closer with skipping turned off, or nothing to skip to.
               (t nil))))))
  nil)

(defun electric-pair--quotes-inhibited ()
  "Whether this mode has asked for quotes not to be paired.

Rust is the reason this exists: `'a' is a lifetime, not the start of a
character literal, so pairing the quote turns every generic parameter into
`''a'. The rule is per-mode because the fact is -- in most languages a lone
quote really does want closing.

Set with (put 'rust-mode 'electric-pair-inhibit-quotes t)."
  (get (major-mode) 'electric-pair-inhibit-quotes))

(defun electric-pair--closing (typed next)
  "Decide what a closing delimiter that was just typed should do.

Three cases, and the middle one is the reason this is a function rather than a
branch. The closer is already in the buffer when this runs -- `self-insert' put
it there -- so deciding not to have typed it means taking it back out.

- The partner is right next to point: step over it.
- The list is closed further along: take the typed one out and go to the
  closer that was already there. Typing `}' a line above an existing one means
  \"finish this block\", and the block is finished -- so this goes to the end of
  it rather than making a second end.
- Neither: leave the typed character where it is.

The take-out-and-check is a real edit, so it is recorded -- but it happens
inside the same command as the insertion it is undoing, so undo sees one step,
not three."
  (cond
   ((and next (string= next typed))
    (progn (delete-backward-char) (forward-char)))
   (t
    (progn
      (delete-backward-char)
      ;; Two things have to be true to call the typed closer surplus: the
      ;; buffer must already balance without it, *and* there must be a list to
      ;; move out of. Balance alone is not enough -- at the top level, with no
      ;; list open at all, the buffer balances trivially and a typed `}' is
      ;; just a character the user wanted.
      (let ((here (point)))
        (if (and (electric-pair--balanced-p)
                 (progn (up-list) (> (point) here)))
            nil
            (progn (goto-char here) (insert typed))))))))

;; ---------------------------------------------------------------------------
;; Deleting
;; ---------------------------------------------------------------------------
;;
;; The one part of this module that is not a hook. Deleting is a command, and
;; the only way to make it aware of pairs is to *be* the command that runs -- so
;; these replace `delete-backward-char' and `delete-char' on their keys and
;; delegate to them in every case but one.
;;
;; That case is stated once and covers both directions: **if the character
;; about to be deleted is half of an empty pair, the whole pair goes.** Four
;; situations fall out of it rather than being enumerated --
;;
;;   ()|  backspace deletes `)', whose partner is just before it
;;   (|)  backspace deletes `(', whose partner is just after point
;;   |()  delete removes `(', whose partner is just after it
;;   (|)  delete removes `)', whose partner is just before it
;;
;; -- and all four leave nothing behind, which is what makes an auto-inserted
;; closer feel undoable rather than permanent.
;;
;; A pair with anything in it is left alone. Deleting the bracket from around
;; text is something people do deliberately, and silently taking the other one
;; with it would destroy the thing they were unwrapping.

(defconst electric-pair-blanks '(" " "\t")
  "What may sit between two delimiters and still count as an empty pair.

Spaces and tabs, deliberately not newlines. `(   )' is a pair someone left a
gap in; `{' and `}' three lines apart is a block with a body about to be
written, and collapsing that on one backspace would be startling.")

(defun electric-pair--blank-p (ch)
  "Whether CH is blank enough to sit inside an otherwise empty pair."
  (and ch (member ch electric-pair-blanks)))

(defun electric-pair--skip-blanks-forward (pos)
  "The first position at or after POS whose character is not blank."
  (let ((p pos))
    (while (and (< p (point-max))
                (electric-pair--blank-p (buffer-substring p (+ p 1))))
      (setq p (+ p 1)))
    p))

(defun electric-pair--skip-blanks-backward (pos)
  "The first position at or before POS whose preceding character is not blank."
  (let ((p pos))
    (while (and (> p (point-min))
                (electric-pair--blank-p (buffer-substring (- p 1) p)))
      (setq p (- p 1)))
    p))

(defun electric-pair--span-around-point ()
  "(START . END) of the empty pair point is *inside*, or nil.

Blanks either side of point are skipped, so this is what catches `(   |   )'
as well as `(|)'. Tried before the character-being-deleted rule below, because
when point sits among the blanks there is no delimiter next to it for that rule
to recognise -- and deleting one space out of the middle of a gap nobody wanted
is not what backspace was pressed for."
  (let ((back (electric-pair--skip-blanks-backward (point)))
        (fwd (electric-pair--skip-blanks-forward (point))))
    (if (and (> back (point-min)) (< fwd (point-max)))
        (let ((opener (buffer-substring (- back 1) back)))
          (if (and (eq (syntax-class opener) 'open)
                   (string= (buffer-substring fwd (+ fwd 1))
                            (matching-delimiter opener)))
              (cons (- back 1) (+ fwd 1))
              nil))
        nil)))

(defun electric-pair--pair-span (pos)
  "(START . END) of the empty pair the character at POS is half of, or nil.

END is one past the closing delimiter, so the span is what `delete-char' would
take given START and (- END START). Blanks between the two are inside the span
and go with it."
  (let ((ch (buffer-substring pos (+ pos 1))))
    (cond
     ((eq (syntax-class ch) 'open)
      (let ((closer-at (electric-pair--skip-blanks-forward (+ pos 1))))
        (if (and (< closer-at (point-max))
                 (string= (buffer-substring closer-at (+ closer-at 1))
                          (matching-delimiter ch)))
            (cons pos (+ closer-at 1))
            nil)))
     ((eq (syntax-class ch) 'close)
      (let ((opener-end (electric-pair--skip-blanks-backward pos)))
        (if (and (> opener-end (point-min))
                 (string= (buffer-substring (- opener-end 1) opener-end)
                          (matching-delimiter ch)))
            (cons (- opener-end 1) (+ pos 1))
            nil)))
     (t nil))))

(defun electric-pair--delete-span (span)
  "Remove SPAN, a (START . END) pair, and leave point where it began."
  (goto-char (car span))
  (delete-char (- (cdr span) (car span))))

(defcommand electric-pair-delete-backward () nil
  "Delete backwards, taking a pair's other half when what goes is half of an
empty one.

Replaces `delete-backward-char' on the backspace key."
  (let ((span (and electric-pair-mode
                   (or (electric-pair--span-around-point)
                       (and (> (point) (point-min))
                            (electric-pair--pair-span (- (point) 1)))))))
    (if span
        (electric-pair--delete-span span)
        (delete-backward-char))))

(defcommand electric-pair-delete-forward () nil
  "Delete forwards, taking a pair's other half when what goes is half of an
empty one.

Replaces `delete-char' on C-d and the delete key -- the mirror of
`electric-pair-delete-backward', and for the same reason: an auto-inserted
closer should be as easy to get rid of from either side."
  (let ((span (and electric-pair-mode
                   (or (electric-pair--span-around-point)
                       (and (< (point) (point-max))
                            (electric-pair--pair-span (point)))))))
    (if span
        (electric-pair--delete-span span)
        (delete-char))))

;; ---------------------------------------------------------------------------
;; Turning it on
;; ---------------------------------------------------------------------------

(defun electric-pair-enable (mode)
  "Pair delimiters in MODE.

Per-mode because `add-hook' is: a hook here belongs to a major mode rather than
being a global variable. That is the right shape for this anyway -- pairing in
a Rust buffer is helpful and pairing in a directory listing is not."
  (add-hook mode "post-self-insert-hook" 'electric-pair-post-self-insert))

(electric-pair-enable 'fundamental-mode)
(electric-pair-enable 'rust-mode)

;; Rust's lifetimes make quote pairing wrong more often than right. See
;; `electric-pair--quotes-inhibited'.
(put 'rust-mode 'electric-pair-inhibit-quotes t)

(define-key nil "<backspace>" 'electric-pair-delete-backward)
;; TODO(fix)
;; For the moment those are disabled because the SyntaxTable is not always
;; computed correctly and opening a bracket the already has its corrispondent
;; closing one far in the buffer is not recoginzed so it is necessary to insert
;; a pair and delete the adiacent closing one
;(define-key nil "C-d" 'electric-pair-delete-forward)
;(define-key nil "<delete>" 'electric-pair-delete-forward)

(log "electric-pair loaded")
