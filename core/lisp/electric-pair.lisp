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

               ;; An opener: put its partner after point and stay between them.
               ((and (eq class 'open) partner)
                (if (not (electric-pair--word-after-p))
                    (progn (insert partner) (backward-char))))

               ;; A closer typed where one already is. The one just inserted is
               ;; removed and point steps over the one that was already there,
               ;; so the buffer ends with one closer rather than two.
               ((and (eq class 'close) electric-pair-skip-self
                     next (string= next typed))
                (progn (delete-backward-char) (forward-char))))))))
  nil)

(defun electric-pair--quotes-inhibited ()
  "Whether this mode has asked for quotes not to be paired.

Rust is the reason this exists: `'a' is a lifetime, not the start of a
character literal, so pairing the quote turns every generic parameter into
`''a'. The rule is per-mode because the fact is -- in most languages a lone
quote really does want closing.

Set with (put 'rust-mode 'electric-pair-inhibit-quotes t)."
  (get (major-mode) 'electric-pair-inhibit-quotes))

;; ---------------------------------------------------------------------------
;; Backspace
;; ---------------------------------------------------------------------------

(defcommand electric-pair-delete-backward () nil
  "Delete backwards, taking a pair's closing delimiter with its opener.

The one part of this module that is not a hook. Deleting is a command of its
own, and the only way to make it aware of pairs is to be the command that runs
-- so this replaces `delete-backward-char' on the backspace key and delegates
to it in every case but one.

That one case is an *empty* pair: `()' with point between. A pair with anything
in it is left alone, because deleting the bracket around text is a thing people
do on purpose."
  (let ((before (electric-pair--char-before))
        (after (electric-pair--char-after)))
    (if (and electric-pair-mode
             before after
             (eq (syntax-class before) 'open)
             (string= after (matching-delimiter before)))
        (progn (delete-char) (delete-backward-char))
        (delete-backward-char))))

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

(log "electric-pair loaded")
