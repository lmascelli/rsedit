;;; electric-pair --- closing delimiters that type themselves.
;;;
;;; Typing an opening delimiter inserts its partner, typing the closing one
;;; steps over it, and deleting half of an empty pair deletes both.
;;; User manual: the `editing' page.
;;; Design: `doc/architettura-editor.typ, <mod-electric-pair>`.

(require 'commands)

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
  ;; A position, not point: the character just typed may have opened the
  ;; string.
  (let ((state (syntax-ppss position)))
    (or (nth 2 state) (nth 3 state))))

(defun electric-pair--closed-ahead-p ()
  "Whether the text after point closes every list that is open at it."
  ;; Depth zero anywhere ahead: see `balance-point'.
  (if (balance-point) t nil))

(defun electric-pair--word-after-p ()
  "Whether the character after point belongs to a word."
  ;; Before a word, a `(' is about to wrap it: no pair.
  (let ((next (electric-pair--char-after)))
    (and next (eq (syntax-class next) 'symbol))))

;; ---------------------------------------------------------------------------
;; The hook
;; ---------------------------------------------------------------------------

(defun electric-pair-post-self-insert ()
  "Pair, or step over, the delimiter just typed, which is before point.

Run from `post-self-insert-hook' in the modes `electric-pair-enable' names."
  ;; Quotes before the in-string test, which would refuse every closing
  ;; quote: just before one, the state is inside its string.
  (if electric-pair-mode
      (let ((typed (electric-pair--char-before)))
        (if typed
            ;; The state before the typed character, which may have opened a
            ;; string or a comment.
            (let* ((state (syntax-ppss (- (point) 1)))
                   (in-string (nth 2 state))
                   (in-comment (nth 3 state))
                   (class (syntax-class typed))
                   (partner (matching-delimiter typed))
                   (next (electric-pair--char-after)))
              (cond
               ;; A comment is prose: nothing in it pairs.
               (in-comment nil)

               ;; A quote opens or closes as what is next to it says.
               ((eq class 'string)
                (cond
                 ((electric-pair--quotes-inhibited) nil)
                 ;; On the partner: step over it.
                 ((and next (string= next typed) electric-pair-skip-self)
                  (progn (delete-backward-char) (forward-char)))
                 ;; Inside a string, not at its end: nothing to open.
                 (in-string nil)
                 ((electric-pair--word-after-p) nil)
                 (t (progn (insert typed) (backward-char)))))

               ;; Everything else leaves the contents of strings alone.
               (in-string nil)

               ;; An opener: its partner after point, unless the list it opens
               ;; is closed already, as when a deleted `{' is typed back.
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
  ;; TYPED is in the buffer already: taken out, within the one command. A `}'
  ;; above one already there goes to the end of the block.
  (cond
   ((and next (string= next typed))
    (progn (delete-backward-char) (forward-char)))
   (t
    (progn
      (delete-backward-char)
      ;; Surplus only if the buffer balances without it and there is a list
      ;; to move out of: at the top level a `}' is a character wanted.
      (let ((here (point)))
        (if (and (electric-pair--closed-ahead-p)
                 (progn (up-list) (> (point) here)))
            nil
            (progn (goto-char here) (insert typed))))))))

;; ---------------------------------------------------------------------------
;; Deleting
;; ---------------------------------------------------------------------------
;;
;; Commands on the keys of `delete-backward-char' and `delete-char', which
;; they call except for half of an empty pair, where the pair goes.

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
  ;; Blanks either side skipped, for `(   |   )' as for `(|)'.
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
  (electric-pair-enable 'cpp-mode)"
  (add-hook mode "post-self-insert-hook" 'electric-pair-post-self-insert))

(electric-pair-enable 'fundamental-mode)
(electric-pair-enable 'rust-mode)

;; Rust's lifetimes make quote pairing wrong more often than right.
(put 'rust-mode 'electric-pair-inhibit-quotes t)

(define-key nil "<backspace>" 'electric-pair-delete-backward)
(define-key nil "C-d" 'electric-pair-delete-forward)
;; TODO(CMD-2): the editor has no name for Delete, and this binding fails.
(define-key nil "<delete>" 'electric-pair-delete-forward)

(log "electric-pair loaded")
(provide 'electric-pair)
