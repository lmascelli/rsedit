;;; electric-pair --- closing delimiters that type themselves.
;;;
;;; Typing an opening delimiter inserts its partner, typing the closing one
;;; steps over it, and deleting half of an empty pair deletes both. User
;;; manual: the `editing' page.
;;;
;;; The pairs come from the mode's syntax table, the one `forward-sexp' and
;;; the indenter read, so there is no second list to keep in step. Pairing
;;; hangs off `post-self-insert-hook', which a paste does not run: pasted code
;;; already has its brackets.

(defvar electric-pair-mode t
  "Whether typing an opening delimiter inserts its partner, and deleting half
of an empty pair deletes both.")

(defvar electric-pair-skip-self t
  "Whether typing a closing delimiter where one already is steps over it,
rather than inserting a second.")

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
  "Whether POSITION is inside a string or a comment."
  ;; A position rather than point: the character just typed may be the one
  ;; that opened the string, which at point would count against pairing it.
  (let ((state (syntax-ppss position)))
    (or (nth 2 state) (nth 3 state))))

(defun electric-pair--closed-ahead-p ()
  "Whether the text after point closes every list that is open at it."
  ;; Reaching depth zero anywhere ahead. Asking whether the innermost list is
  ;; closed would refuse to pair a `{' typed inside a closed block, since a
  ;; closer matches the innermost opener; asking whether the whole buffer
  ;; balances would add a second brace whenever anything further down is
  ;; unfinished. Stopping at the first balance also keeps the scan within the
  ;; outermost list point is in.
  (if (balance-point) t nil))

(defun electric-pair--word-after-p ()
  "Whether the character after point belongs to a word."
  ;; Pairing before a word is nearly always wrong: `(' typed in front of `foo'
  ;; is about to wrap it. Emacs makes this configurable; here it is the rule.
  (let ((next (electric-pair--char-after)))
    (and next (eq (syntax-class next) 'symbol))))

;; ---------------------------------------------------------------------------
;; The hook
;; ---------------------------------------------------------------------------

(defun electric-pair-post-self-insert ()
  "Pair, or step over, the delimiter just typed, which is before point.

Run from `post-self-insert-hook' in the modes `electric-pair-enable' names."
  ;; The order of the tests below is load-bearing. Quotes are decided before
  ;; the "not inside a string" rule, because for a quote that rule would
  ;; refuse every closing quote: the state just before a closing quote is
  ;; inside the string it closes.
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

               ;; Quotes, before the in-string test. Which of open and close
               ;; this is cannot be read off the character -- the same one
               ;; does both -- so it is read off what is next to it.
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
                        (electric-pair--closed-ahead-p))
                    nil
                    (progn (insert partner) (backward-char))))

               ;; A closer, where one is already doing the job.
               ((and (eq class 'close) electric-pair-skip-self)
                (electric-pair--closing typed next))

               ;; A closer with skipping turned off, or nothing to skip to.
               (t nil))))))
  nil)

(defun electric-pair--quotes-inhibited ()
  "Whether this mode has an electric-pair-inhibit-quotes property."
  (get (major-mode) 'electric-pair-inhibit-quotes))

(defun electric-pair--closing (typed next)
  "Step over NEXT when it is the closer TYPED, go to the closer already ahead,
or leave TYPED where it is."
  ;; TYPED is already in the buffer, so not having typed it means taking it
  ;; back out. Typing `}' a line above an existing one means "finish this
  ;; block", and the block is finished, so point goes to its end. Taking out
  ;; and putting back happen within the one command, so undo sees one step.
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
        (if (and (electric-pair--closed-ahead-p)
                 (progn (up-list) (> (point) here)))
            nil
            (progn (goto-char here) (insert typed))))))))

;; ---------------------------------------------------------------------------
;; Deleting
;; ---------------------------------------------------------------------------
;;
;; The one part of this module that is not a hook. Deleting is a command, so
;; these replace `delete-backward-char' and `delete-char' on their keys and
;; call them in every case but one: when the character about to go is half
;; of an empty pair, the whole pair goes.

(defconst electric-pair-blanks '(" " "\t")
  "What may sit between two delimiters that still count as an empty pair:
spaces and tabs, never a newline.")

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
  "(START . END) of the empty pair point is inside, or nil."
  ;; Blanks either side of point are skipped, so this catches `(   |   )' as
  ;; well as `(|)'. Tried before the rule for the character being deleted,
  ;; since among the blanks there is no delimiter next to point to recognise.
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
  "(START . END) of the empty pair the character at POS is half of, or nil."
  ;; END is one past the closing delimiter, and the blanks between the two
  ;; are inside the span.
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
  "Delete the character before point, or the whole of the empty pair it is
half of."
  (let ((span (and electric-pair-mode
                   (or (electric-pair--span-around-point)
                       (and (> (point) (point-min))
                            (electric-pair--pair-span (- (point) 1)))))))
    (if span
        (electric-pair--delete-span span)
        (delete-backward-char))))

(defcommand electric-pair-delete-forward () nil
  "Delete the character after point, or the whole of the empty pair it is
half of."
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
  "Pair delimiters in the buffers of MODE.

Example:
  (electric-pair-enable 'cc-mode)"
  (add-hook mode "post-self-insert-hook" 'electric-pair-post-self-insert))

(electric-pair-enable 'fundamental-mode)
(electric-pair-enable 'rust-mode)

;; Rust's lifetimes make quote pairing wrong more often than right.
(put 'rust-mode 'electric-pair-inhibit-quotes t)

(define-key nil "<backspace>" 'electric-pair-delete-backward)
(define-key nil "C-d" 'electric-pair-delete-forward)
(define-key nil "<delete>" 'electric-pair-delete-forward)

(log "electric-pair loaded")
