;;; vim --- modal editing, as Vim does it.
;;;
;;; M-x vim-mode gives a buffer Vim's keys, and global-vim-mode gives them to
;;; every buffer that suits. User manual: the `vim' page.
;;;
;;; Load after commands and debug. Syntax, indentation, major-mode hooks and
;;; prompts keep their own modes: a state is a local keymap over the buffer's
;;; own. Registers, searches, dot and macros are shared; modal input is
;;; buffer-local.

(defvar vim-shift-width 4 "How many columns > and < shift a line by.")
(defvar vim-search-wrap t
  "Whether a Vim search goes on from the other end of the buffer when it
reaches one. :set nowrapscan sets it to nil.")
(defvar vim-registers nil
  "Vim's registers, shared by every buffer: a list of (NAME TEXT TYPE), NAME a
string such as \"a\" and TYPE char, line, block or macro.")
(defvar vim-last-change nil "The edit . repeats, in a form of Vim's own.")
(defvar vim-search-pattern nil
  "The pattern the last Vim search looked for, which n, N and an empty / look
for again.")
(defvar vim-search-direction 1
  "1 when the last Vim search went forward, -1 when it went backward.")
(defvar vim-search-regexp t
  "Whether the last Vim search pattern is a regular expression; every search
Vim makes sets it to t.")
(defvar vim-last-find nil
  "The last f, F, t or T, as (KEY CHARACTER), which ; and , repeat.")
(defvar vim-recording-register nil
  "The register q is recording keys into, or nil.")
(defvar vim-last-macro-register nil
  "The register the last @ played, which @@ plays again.")
(defvar vim--replaying nil
  "Non-nil while . replays an edit, so that the replay is not recorded in its
place.")
(defvar global-vim-mode nil
  "Whether Vim turns itself on in every buffer that suits it. The command of
the same name sets it, and applies it.")
(defvar vim-mode-modes nil
  "The major modes Vim turns itself on in, whatever `global-vim-mode' says: a
list of symbols. Run `vim-mode-refresh' after changing it.")
(defvar vim-mode-excluded-modes
  '(minibuffer-mode completion-mode dired-mode help-mode manpage-mode
    backtrace-mode buffer-list-mode theme-list-mode shell-output-mode occur-mode compilation-mode)
  "The major modes Vim never turns itself on in: a list of symbols, at first
the editor's listings and buffers of its own. M-x vim-mode still turns it on
there.")
(defvar vim-mode-predicate 'vim-default-buffer-p
  "The function, of no arguments, that says whether the current buffer suits
automatic Vim, or nil for every buffer. At first `vim-default-buffer-p'.")

(defun vim--min (a b)
  "The smaller of A and B."
  (if (< a b) a b))
(defun vim--max (a b)
  "The larger of A and B."
  (if (> a b) a b))
(defun vim--state ()
  "This buffer's Vim state: normal, insert, replace, visual, visual-line or
visual-block, or nil when Vim is off."
  (buffer-get 'vim-state))
(defun vim--visual-p ()
  "Whether this buffer is in one of the three Visual states."
  (member (vim--state) '(visual visual-line visual-block)))
(defun vim--char (at)
  "The character at AT, as a string, or \"\" outside the buffer."
  (if (and (>= at 0) (< at (point-max))) (buffer-substring at (+ at 1)) ""))
(defun vim--space-p (c)
  "Whether the character C is white space, or \"\"."
  (or (string= c "") (string-match-p "^[[:space:]]$" c)))
(defun vim--class (at big)
  "The class of the character at AT for the word motions: 0 for white space,
1 for a word character, 2 for punctuation; with BIG, 1 for both."
  (let ((c (vim--char at)))
    (cond ((vim--space-p c) 0)
          (big 1)
          ((string-match-p "^[[:alnum:]_]$" c) 1)
          (t 2))))
(defun vim--bol (at)
  "Where the line holding AT begins."
  (let ((saved (point)))
    (goto-char at) (beginning-of-line)
    (let ((answer (point))) (goto-char saved) answer)))
(defun vim--eol (at)
  "Where the line holding AT ends, before its newline."
  (let ((saved (point)))
    (goto-char at) (end-of-line)
    (let ((answer (point))) (goto-char saved) answer)))
(defun vim--line-end (at)
  "Where the line holding AT ends, after its newline."
  (vim--min (point-max) (+ (vim--eol at) 1)))
(defun vim--line-number (at)
  "The number of the line holding AT, from 1."
  (let ((saved (point)))
    (goto-char at)
    (let ((line (line-number-at-point))) (goto-char saved) line)))
(defun vim--first (at)
  "The first character of the line holding AT that is not a space or a tab."
  (let ((start (vim--bol at)) (end (vim--eol at)))
    (while (and (< start end) (member (vim--char start) '(" " "\t")))
      (setq start (+ start 1)))
    start))
(defun vim--clamp ()
  "Put point back on the last character of its line, where Normal state keeps
it, unless the line is empty."
  (let ((end (vim--eol (point))))
    (when (and (= (point) end) (> end (vim--bol (point))))
      (goto-char (- end 1)))))
(defun vim--reset ()
  "Forget the count, the operator, the register and the key waiting for the
rest of a command."
  (dolist (key '(vim-count vim-operator vim-op-count vim-pending vim-register vim-object))
    (buffer-put key nil)))
(defun vim--set-state (state)
  "Put this buffer in Vim state STATE, with its keymap and its name on the
status line."
  (buffer-put 'vim-state state)
  (set-local-keymap (cond ((eq state 'insert) 'vim-insert-map) ((eq state 'replace) 'vim-replace-map) (t 'vim-normal-map)))
  (buffer-put 'mode-line-state
              (cond ((eq state 'insert) "INSERT")
                    ((eq state 'replace) "REPLACE")
                    ((eq state 'visual) "VISUAL")
                    ((eq state 'visual-line) "V-LINE")
                    ((eq state 'visual-block) "V-BLOCK")
                    (t "NORMAL")))
  (set-echo-message (concat "-- " (buffer-get 'mode-line-state) " --")))

(defun vim--enable ()
  "Turn Vim on in this buffer; when it is on, keep its state, and put its
keymap back if a mode replaced it."
  (if (vim--state)
      (let ((map (cond ((eq (vim--state) 'insert) 'vim-insert-map)
                       ((eq (vim--state) 'replace) 'vim-replace-map)
                       (t 'vim-normal-map))))
        ;; A major-mode hook may have installed a new underlying local map.
        (when (not (eq (buffer-get 'local-keymap) map))
          (buffer-put 'vim-saved-keymap (buffer-get 'local-keymap))
          (set-local-keymap map)))
      (progn
        (buffer-put 'vim-saved-keymap (buffer-get 'local-keymap))
        (buffer-put 'vim-saved-mode-line-state (buffer-get 'mode-line-state))
        (vim--reset) (deactivate-mark) (vim--set-state 'normal) (vim--clamp))))
(defun vim--disable ()
  "Turn Vim off in this buffer, giving back the keymap and the status line it
had."
  (when (vim--state)
    (when (member (vim--state) '(insert replace)) (vim--finish-insert))
    (when (member (buffer-get 'local-keymap) '(vim-normal-map vim-insert-map vim-replace-map))
      (set-local-keymap (buffer-get 'vim-saved-keymap)))
    (buffer-put 'mode-line-state (buffer-get 'vim-saved-mode-line-state))
    (dolist (key '(vim-state vim-anchor vim-insert-before vim-insert-action vim-insert-start))
      (buffer-put key nil))
    (remove-overlays 'vim-selection)
    (vim--reset) (deactivate-mark) (set-echo-message "Vim mode disabled")))

(defun vim--turn-on-p (enabled on)
  "Whether ENABLED, given to a mode that is ON now, leaves it on."
  ;; ENABLED is a value from Lisp or the raw prefix argument: nil toggles,
  ;; and - is what C-u - gives, minus one as in Emacs.
  (cond ((null enabled) (not on))
        ((eq enabled '-) nil)
        ((numberp enabled) (> enabled 0))
        (t t)))
(defcommand vim-mode (&optional enabled) ("P")
  "Turn Vim's keys on in this buffer, or off when they are on. With ENABLED,
or a prefix argument, 0, a negative number or the - of C-u - turns them off,
and anything else on. A buffer turned on or off so keeps its choice whatever
the automatic rules say, until `vim-mode-auto'."
  (if (vim--turn-on-p enabled (vim--state))
      (progn (buffer-put 'vim-mode-override 'enabled) (vim--enable))
      (vim-mode-disable)))
(defcommand vim-mode-enable () nil
  "Turn Vim on in this buffer; when it is on already, keep its state, an
Insert session or a command half typed."
  (vim-mode t))
(defcommand vim-mode-disable () nil
  "Turn Vim off in this buffer, and give it back the keymap it had before."
  (buffer-put 'vim-mode-override 'disabled)
  (vim--disable))
(defun vim-default-buffer-p ()
  "Non-nil when the current buffer suits automatic Vim: it visits a file, is
*scratch*, or has a name that does not start with a *, as the editor's own
buffers do. The first value of `vim-mode-predicate'."
  (or (buffer-file-name) (string= (current-buffer) "*scratch*")
      (not (string-prefix-p "*" (current-buffer)))))
(defun vim--apply-policy ()
  "Turn Vim on or off in this buffer as the automatic rules say, unless it
was turned on or off by hand."
  (let ((preference (buffer-get 'vim-mode-override)))
    (cond ((eq preference 'enabled) (vim--enable))
          ((eq preference 'disabled) (vim--disable))
          ((and (not (string= (current-buffer) "*Minibuffer*"))
                (or global-vim-mode (member (major-mode) vim-mode-modes))
                (not (member (major-mode) vim-mode-excluded-modes))
                (or (not vim-mode-predicate) (funcall vim-mode-predicate)))
           (vim--enable))
          (t (vim--disable)))))
(defcommand vim-mode-refresh () nil
  "Apply the automatic rules again to every buffer, after `global-vim-mode',
`vim-mode-modes', `vim-mode-excluded-modes' or `vim-mode-predicate' has
changed. A buffer turned on or off by hand stays so."
  (dolist (name (all-buffer-names))
    (when (major-mode name)
      (with-current-buffer name (lambda () (vim--apply-policy))))))
(defcommand vim-mode-auto () nil
  "Give this buffer back to the automatic rules, after `vim-mode',
`vim-mode-enable' or `vim-mode-disable' turned it on or off, and apply them."
  (buffer-put 'vim-mode-override nil)
  (vim--apply-policy))
(defcommand global-vim-mode (&optional enabled) ("P")
  "Turn Vim on in every buffer that suits it, those there are and those to
come, or off when it is on. With ENABLED, or a prefix argument, 0, a negative
number or the - of C-u - turns it off, and anything else on. A buffer turned
on by hand, and one whose major mode is in `vim-mode-modes', keeps Vim either
way."
  (setq global-vim-mode (vim--turn-on-p enabled global-vim-mode))
  (vim-mode-refresh)
  global-vim-mode)
(defcommand vim-normal-state () nil
  "Go back to Normal state: leave Insert, Replace or Visual state, or cancel
a command half typed."
  (when (member (vim--state) '(insert replace)) (vim--finish-insert))
  (remove-overlays 'vim-selection)
  (vim--reset) (buffer-put 'vim-anchor nil)
  (deactivate-mark) (vim--set-state 'normal) (vim--clamp))
(defun vim--number ()
  "The count typed before the command, or 1."
  (or (buffer-get 'vim-count) 1))
(defun vim--digit (key)
  "Add the digit KEY to the count being typed."
  (buffer-put 'vim-count (+ (* (or (buffer-get 'vim-count) 0) 10) (string-to-number key)))
  (set-echo-message (number-to-string (buffer-get 'vim-count))))
(defun vim--remember (action)
  "Remember ACTION as the edit . repeats, unless . is repeating one."
  (when (not vim--replaying) (setq vim-last-change (list action nil))))

;;; Motions return (POSITION INCLUSIVE LINEWISE). Failed motions return nil.
(defun vim--word-forward (at count big)
  "Where the word COUNT words after AT begins; with BIG, the WORD."
  (dotimes (i count)
    (let ((class (vim--class at big)))
      (when (> class 0)
        (while (and (< at (point-max)) (= (vim--class at big) class))
          (setq at (+ at 1))))
      (while (and (< at (point-max)) (= (vim--class at big) 0))
        (setq at (+ at 1)))))
  at)
(defun vim--word-backward (at count big)
  "Where the word COUNT words before AT begins; with BIG, the WORD."
  (dotimes (i count)
    (setq at (vim--max 0 (- at 1)))
    (while (and (> at 0) (= (vim--class at big) 0)) (setq at (- at 1)))
    (let ((class (vim--class at big)))
      (while (and (> at 0) (= (vim--class (- at 1) big) class))
        (setq at (- at 1)))))
  at)
(defun vim--word-end (at count big)
  "Where the word COUNT words from AT ends; with BIG, the WORD."
  (dotimes (i count)
    (setq at (vim--min (point-max) (+ at 1)))
    (while (and (< at (point-max)) (= (vim--class at big) 0)) (setq at (+ at 1)))
    (let ((class (vim--class at big)))
      (while (and (< (+ at 1) (point-max)) (= (vim--class (+ at 1) big) class))
        (setq at (+ at 1)))))
  (vim--min at (vim--max 0 (- (point-max) 1))))
(defun vim--find (key c count)
  "The motion of f, F, t or T, KEY, to the COUNTth C on this line, or nil."
  (let ((at (point)) (step (if (member key '("f" "t")) 1 -1))
        (start (vim--bol (point))) (end (vim--eol (point))) (found t))
    (dotimes (i count)
      (setq at (+ at step))
      (while (and (>= at start) (< at end) (not (string= (vim--char at) c)))
        (setq at (+ at step)))
      (when (or (< at start) (>= at end)) (setq found nil)))
    (when found
      (when (member key '("t" "T")) (setq at (- at step)))
      (list at t nil))))
(defun vim--match-pair ()
  "The motion of %, to the bracket matching the first one from point on its
line, or nil."
  (let ((at (point)) (end (vim--eol (point))) (open nil))
    (while (and (< at end) (not open))
      (when (member (vim--char at) '("(" ")" "[" "]" "{" "}")) (setq open at))
      (when (not open) (setq at (+ at 1))))
    (when open
      (let* ((c (vim--char open)) (partner (matching-delimiter c))
             (step (if (member c '("(" "[" "{")) 1 -1)) (depth 1))
        (setq at (+ open step))
        (while (and (>= at 0) (< at (point-max)) (> depth 0))
          (cond ((string= (vim--char at) c) (setq depth (+ depth 1)))
                ((string= (vim--char at) partner) (setq depth (- depth 1))))
          (when (> depth 0) (setq at (+ at step))))
        (when (= depth 0) (list at t nil))))))
(defun vim--motion (key count &optional argument)
  "Where the motion KEY goes COUNT times, as (POSITION INCLUSIVE LINEWISE),
or nil; ARGUMENT is the character of f, F, t and T."
  (let ((at (point)) (answer nil))
    (setq answer
      (cond
        ((member key '("h" "<left>" "<backspace>"))
         (list (vim--max (vim--bol at) (- at count)) nil nil))
        ((member key '("l" "<right>" "<space>"))
         (list (vim--min (vim--eol at) (+ at count)) nil nil))
        ((member key '("j" "k" "<down>" "<up>" "+" "-" "<ret>"))
         (let ((column (or (buffer-get 'vim-goal-column) (- at (vim--bol at)))))
           (buffer-put 'vim-goal-column column)
           (goto-char (vim--bol at))
           (next-line (* count (if (member key '("k" "<up>" "-")) -1 1)))
           (setq answer (point))
           (setq answer (if (member key '("+" "-" "<ret>"))
                            (vim--first answer)
                            (+ answer (vim--min column (- (vim--eol answer) answer)))))
           (list answer nil t)))
        ((string= key "0") (list (vim--bol at) nil nil))
        ((member key '("^" "_")) (list (vim--first at) nil nil))
        ((string= key "$")
         (goto-char (vim--bol at)) (next-line (- count 1))
         (list (vim--max (vim--bol (point)) (- (vim--eol (point)) 1))
               (> (vim--eol (point)) (vim--bol (point))) nil))
        ((string= key "|") (list (+ (vim--bol at) (vim--min (- count 1) (- (vim--eol at) (vim--bol at)))) nil nil))
        ((member key '("w" "W")) (list (vim--word-forward at count (string= key "W")) nil nil))
        ((member key '("b" "B")) (list (vim--word-backward at count (string= key "B")) nil nil))
        ((member key '("e" "E")) (list (vim--word-end at count (string= key "E")) t nil))
        ((string= key "ge")
         ;; Back past the rest of the word point is in, which is nothing on its
         ;; first character, then over the white space before it.
         (let ((target at))
           (dotimes (i count)
             (let ((class (vim--class target nil)))
               (setq target (vim--max 0 (- target 1)))
               (when (> class 0)
                 (while (and (> target 0) (= (vim--class target nil) class))
                   (setq target (- target 1))))
               (while (and (> target 0) (= (vim--class target nil) 0))
                 (setq target (- target 1)))))
           (list target t nil)))
        ((member key '("gg" "G"))
         (goto-line (if (string= key "gg") count
                       (if (or (buffer-get 'vim-count) (> count 1)) count (vim--line-number (vim--max 0 (- (point-max) 1))))))
         (list (vim--first (point)) nil t))
        ((member key '("n" "N"))
         (when (vim--search-next (* vim-search-direction (if (string= key "n") 1 -1)) count)
           (list (point) nil nil)))
        ((member key '("*" "#"))
         (when (vim--search-word key count) (list (point) nil nil)))
        ((member key '("{" "}"))
         (dotimes (i count)
           (if (string= key "}") (forward-paragraph) (backward-paragraph)))
         (list (point) nil nil))
        ((string= key "%") (vim--match-pair))
        ((member key '("f" "F" "t" "T")) (vim--find key argument count))
        ((member key '(";" ","))
         (when vim-last-find
           (let ((find-key (car vim-last-find)))
             (when (string= key ",")
               (setq find-key (cond ((string= find-key "f") "F") ((string= find-key "F") "f")
                                    ((string= find-key "t") "T") (t "t"))))
             ;; Repeated till must step past its already reached boundary.
             (when (member find-key '("t" "T"))
               (goto-char (+ at (if (string= find-key "t") 1 -1))))
             (vim--find find-key (nth 1 vim-last-find) count))))
        (t nil)))
    (goto-char at)
    (when (not (member key '("j" "k" "<down>" "<up>"))) (buffer-put 'vim-goal-column nil))
    answer))

;;; Registers retain the shape of text: character, line or block.
(defun vim--register-value (name)
  "The value of register NAME, (TEXT TYPE), or nil."
  (cdr (assoc name vim-registers)))
(defun vim--put-register (name value)
  "Set register NAME to VALUE, (TEXT TYPE)."
  (setq vim-registers (cons (cons name value)
                          (let ((remaining vim-registers) (kept nil))
                            (dolist (entry remaining)
                              (when (not (string= (car entry) name)) (setq kept (cons entry kept))))
                            (reverse kept)))))
(defun vim--save-text (text type operation)
  "Save TEXT, of shape TYPE, deleted or yanked by OPERATION, in the registers
Vim saves it in: the one named, the unnamed one, and 0, 1 to 9 or -."
  (let ((name (or (buffer-get 'vim-register) "\"")))
    (when (not (string= name "_"))
      (when (string-match-p "^[A-Z]$" name)
        (setq name (downcase name))
        (let ((old (vim--register-value name)))
          (when old (setq text (concat (car old) text)))))
      (let ((value (list text type)))
        (vim--put-register name value)
        (vim--put-register "\"" value)
        (if (string= operation "y")
            (vim--put-register "0" value)
            (if (or (eq type 'line) (string-match-p "\n" text))
                (progn
                  (let ((index 9))
                    (while (> index 1)
                      (let ((old (vim--register-value (number-to-string (- index 1)))))
                        (when old (vim--put-register (number-to-string index) old)))
                      (setq index (- index 1))))
                  (vim--put-register "1" value))
                (vim--put-register "-" value))))
      (kill-new text))))
(defun vim--line-range (from to)
  "The whole lines from FROM to TO, as (START END line)."
  (list (vim--bol (vim--min from to)) (vim--line-end (vim--max from to)) 'line))
(defun vim--range (from motion)
  "What an operator acts on from FROM to where MOTION goes, as (START END
TYPE)."
  (let ((to (car motion)))
    (if (nth 2 motion) (vim--line-range from to)
        (list (vim--min from to)
              (vim--min (point-max) (+ (vim--max from to) (if (nth 1 motion) 1 0))) 'char))))
(defun vim--visual-range ()
  "The Visual selection, as (START END TYPE)."
  (let ((anchor (buffer-get 'vim-anchor)))
    (if (eq (vim--state) 'visual-line) (vim--line-range anchor (point))
        (list (vim--min anchor (point)) (vim--min (point-max) (+ (vim--max anchor (point)) 1)) 'char))))
(defun vim--show-visual ()
  "Draw the Visual selection: the region, a rectangle, or an overlay on its
lines."
  (remove-overlays 'vim-selection)
  (when (vim--visual-p)
    (set-mark (buffer-get 'vim-anchor) t)
    (cond ((eq (vim--state) 'visual-block) (rectangle-mark-mode))
          ((eq (vim--state) 'visual-line)
           (let ((range (vim--visual-range)))
             (make-overlay (car range) (nth 1 range) 'region 100 'vim-selection))))))
(defun vim--visual (state)
  "Enter Visual state STATE, or go back to Normal state when already in it."
  (if (eq (vim--state) state) (vim-normal-state)
      (progn
        (when (not (vim--visual-p)) (buffer-put 'vim-anchor (point)))
        (vim--set-state state) (vim--show-visual))))

;;; Text objects: words/WORDS, balanced delimiters, quotes, paragraphs.
(defun vim--white-line-p (at)
  "Whether the line holding AT is empty, or white space only."
  (string-match-p "^[[:space:]]*$" (buffer-substring (vim--bol at) (vim--eol at))))
(defun vim--paragraphs (at count around)
  "The lines of ip, or of ap with AROUND, COUNT times from AT, as (START END),
or nil when the buffer ends first."
  ;; Vim's rules. For ip a run of blank lines counts as one, as a paragraph
  ;; does; ap takes a paragraph with the blank lines after it, and on blank
  ;; lines the blank lines with the paragraph after them. When ap ends on a
  ;; paragraph, at the end of the buffer, it takes the blank lines before
  ;; the first one instead.
  (let* ((white (vim--white-line-p at)) (start (vim--bol at)) (end nil)
         (times (if (and white (not around)) (- count 1) count)) (failed nil))
    (while (and (> start 0) (eq (not (vim--white-line-p (- start 1))) (not white)))
      (setq start (vim--bol (- start 1))))
    (setq end start)
    (while (and (< end (point-max)) (vim--white-line-p end))
      (setq end (vim--line-end end)))
    (dotimes (i times)
      (when (not failed)
        (if (>= end (point-max))
            (setq failed t)
            (let ((blank (and (not around) (vim--white-line-p end))))
              (when (not blank)
                (setq end (vim--line-end end))
                (while (and (< end (point-max)) (not (vim--white-line-p end)))
                  (setq end (vim--line-end end))))
              (when (and (or around blank) (not (and around white (= i (- times 1)))))
                (while (and (< end (point-max)) (vim--white-line-p end))
                  (setq end (vim--line-end end))))))))
    (when (and around (not white) (not failed) (not (vim--white-line-p (- end 1))))
      (while (and (> start 0) (vim--white-line-p (- start 1)))
        (setq start (vim--bol (- start 1)))))
    (when (not failed) (list start end))))
(defun vim--object (key around count)
  "The text object KEY around point, COUNT of them, as (START END TYPE), or
nil; AROUND for a, not i."
  (let ((at (point)) (start nil) (end nil) (type 'char))
    (cond
      ((member key '("w" "W"))
       (let ((big (string= key "W")) (class (vim--class at (string= key "W"))))
         (setq start at end at)
         (while (and (> start 0) (= (vim--class (- start 1) big) class)) (setq start (- start 1)))
         (while (and (< end (point-max)) (= (vim--class end big) class)) (setq end (+ end 1)))
         (when (> count 1) (setq end (+ (vim--word-end (vim--max start (- end 1)) (- count 1) big) 1)))
         (when around
           (let ((old-end end))
             (while (and (< end (point-max)) (member (vim--char end) '(" " "\t"))) (setq end (+ end 1)))
             (when (= old-end end)
               (while (and (> start 0) (member (vim--char (- start 1)) '(" " "\t"))) (setq start (- start 1))))))))
      ((member key '("(" ")" "b" "[" "]" "{" "}" "B" "<" ">"))
       (let* ((open (cond ((member key '("(" ")" "b")) "(")
                          ((member key '("[" "]")) "[")
                          ((member key '("{" "}" "B")) "{") (t "<")))
              (close (cond ((string= open "(") ")") ((string= open "[") "]")
                           ((string= open "{") "}") (t ">")))
              (scan at) (depth 0))
         (while (and (>= scan 0) (not start))
           (cond ((string= (vim--char scan) open)
                  (if (= depth 0) (setq start scan) (setq depth (- depth 1))))
                 ((and (< scan at) (string= (vim--char scan) close)) (setq depth (+ depth 1))))
           (setq scan (- scan 1)))
         (when start
           (setq scan (+ start 1) depth 1)
           (while (and (< scan (point-max)) (> depth 0))
             (cond ((string= (vim--char scan) open) (setq depth (+ depth 1)))
                   ((string= (vim--char scan) close) (setq depth (- depth 1))))
             (when (> depth 0) (setq scan (+ scan 1))))
           (if (= depth 0)
               (setq end (if around (+ scan 1) scan) start (if around start (+ start 1)))
               (setq start nil)))))
      ((member key '("\"" "'" "`"))
       (let ((scan (vim--bol at)) (limit (vim--eol at)) (candidate nil))
         (while (< scan limit)
           (when (and (string= (vim--char scan) key)
                      (not (string= (vim--char (- scan 1)) "\\")))
             (if candidate
                 (progn
                   (when (and (<= candidate at) (>= scan at) (not start))
                     (setq start (if around candidate (+ candidate 1))
                           end (if around (+ scan 1) scan)))
                   (setq candidate nil))
                 (setq candidate scan)))
           (setq scan (+ scan 1)))))
      ((string= key "p")
       (let ((lines (vim--paragraphs at count around)))
         (when lines (setq start (car lines) end (nth 1 lines) type 'line)))))
    (goto-char at)
    (when start (list start end type))))

(defun vim--begin-insert (action)
  "Enter Insert state for ACTION, remembering the text, so that leaving can
tell what was typed."
  (buffer-put 'vim-insert-before (buffer-string))
  (buffer-put 'vim-insert-start (point))
  (buffer-put 'vim-insert-action action)
  (buffer-put 'vim-insert-join nil)
  (vim--set-state 'insert))
(defun vim--pre-command ()
  "Before each command of an Insert session, join its edits into one undo
step."
  (when (and (member (vim--state) '(insert replace)) (buffer-get 'vim-insert-join)) (undo-join)))
(defun vim--post-command ()
  "After each command, mark an Insert session as under way, and draw the
Visual selection again."
  (when (member (vim--state) '(insert replace)) (buffer-put 'vim-insert-join t))
  (when (not (eq (vim--state) 'visual-line)) (remove-overlays 'vim-selection))
  (when (vim--visual-p) (vim--show-visual)))
(defun vim--finish-insert ()
  "Leave an Insert or Replace session: type a counted insertion again, and
record what changed for ."
  (let ((before (buffer-get 'vim-insert-before)) (action (buffer-get 'vim-insert-action)))
    (when before
      (let* ((change (string-diff before (buffer-string)))
             (offset (- (car change) (buffer-get 'vim-insert-start)))
             (removed (nth 1 change)) (inserted (nth 2 change))
             (count (if (member (car action) '(insert open)) (nth 2 action) 1)))
        (when (and (> count 1) (> (length inserted) 0) (= removed 0))
          (undo-join)
          (dotimes (i (- count 1))
            (when (eq (car action) 'open) (insert "\n" (make-string (current-indentation) " ")))
            (insert inserted)))
        (when (buffer-get 'vim-block-insert)
          (undo-join) (vim--replicate-block-insert inserted))
        (when (and (not vim--replaying)
                   (or (> removed 0) (> (length inserted) 0) (member (car action) '(operator object open shortcut block))))
          (setq vim-last-change (list action (list offset removed inserted))))))
    (buffer-put 'vim-insert-before nil)
    (buffer-put 'vim-insert-join nil)
    (when (> (point) (vim--bol (point))) (goto-char (- (point) 1)))))
(defun vim--insert-command (key count)
  "Run i, a, I, A, o or O, KEY, COUNT times: move, open a line, and enter
Insert state."
  (when (not (buffer-read-only-p))
    (cond ((string= key "a") (goto-char (vim--min (vim--eol (point)) (+ (point) 1))))
          ((string= key "I") (goto-char (vim--first (point))))
          ((string= key "A") (goto-char (vim--eol (point))))
          ((member key '("o" "O"))
           (let ((indent (current-indentation)))
             (if (string= key "o")
                 (progn (goto-char (vim--eol (point))) (insert "\n"))
                 (progn (goto-char (vim--bol (point))) (insert "\n") (goto-char (- (point) 1))))
             (insert (make-string indent " ")))))
    (vim--begin-insert (list (if (member key '("o" "O")) 'open 'insert) key count))
    (when (member key '("o" "O")) (buffer-put 'vim-insert-join t))))
(defun vim--apply-range (operation range action)
  "Apply the operator OPERATION to RANGE, (START END TYPE), recording ACTION
for ."
  (let ((start (car range)) (end (nth 1 range)) (type (nth 2 range)))
    (when (and range (or (string= operation "y") (not (buffer-read-only-p))))
      (cond
        ((member operation '("d" "c" "y"))
         (when (< start end) (vim--save-text (buffer-substring start end) type operation))
         (when (not (string= operation "y"))
           (let ((delete-start start) (delete-end end))
             ;; Removing the last physical line consumes its preceding newline.
             (when (and (eq type 'line) (= end (point-max)) (> start 0)
                        (not (string= operation "c")) (not (string= (vim--char (- end 1)) "\n")))
               (setq delete-start (- start 1)))
             (delete-region delete-start delete-end)
             (goto-char delete-start)
             (when (and (eq type 'line) (string= operation "c") (< start (point-max)))
               (insert "\n") (goto-char start)))
           (when (< start end) (vim--remember action))))
        ((member operation '(">" "<" "="))
         (let ((last (vim--bol (vim--max start (- end 1)))) (at (vim--bol start)))
           (goto-char last)
           (while (>= (point) at)
             (let ((line-start (point)))
               (cond ((string= operation "=")
                      (when (functionp 'indent-line) (indent-line)))
                     (t (indent-line-to
                          (vim--max 0 (+ (current-indentation)
                                         (* vim-shift-width (if (string= operation ">") 1 -1)))))))
               (if (= line-start at) (goto-char 0)
                   (goto-char (vim--bol (- line-start 1))))
               (when (= line-start at) (setq at 1) (goto-char 0))))
           (goto-char start))
         (vim--remember action))
        ((member operation '("gU" "gu" "g~" "~"))
         (let ((text (buffer-substring start end)))
           (delete-region start end) (goto-char start)
           (insert (cond ((string= operation "gU") (upcase text))
                         ((string= operation "gu") (downcase text))
                         (t (vim--toggle-case text))))
           (goto-char start))
         (vim--remember action)))
      (when (vim--visual-p)
        (goto-char start) (vim--set-state 'normal) (deactivate-mark) (remove-overlays 'vim-selection))
      (if (string= operation "c")
          (progn (vim--begin-insert action) (buffer-put 'vim-insert-join t))
          (vim--clamp)))))
(defun vim--toggle-case (text)
  "TEXT with the case of every letter swapped."
  (let ((result ""))
    (dotimes (i (length text))
      (let ((c (substring text i (+ i 1))))
        (setq result (concat result (if (string= c (upcase c)) (downcase c) (upcase c))))))
    result))
(defun vim--operator-motion (key count &optional argument)
  "Apply the pending operator to where the motion KEY goes COUNT times; the
operator's own key again means whole lines."
  (let* ((operation (buffer-get 'vim-operator)) (from (point))
         (motion-key (if (and (string= operation "c") (member key '("w" "W"))
                             (> (vim--class from (string= key "W")) 0))
                         (if (string= key "w") "e" "E") key))
         (motion (if (and (string= operation "c") (member key '("w" "W"))
                          (> (vim--class from (string= key "W")) 0))
                     (list (vim--word-end (- from 1) count (string= key "W")) t nil)
                     (if (or (string= key operation)
                             (and (member operation '("gU" "gu" "g~")) (string= key (substring operation 1))))
                     (progn (goto-char (vim--bol from)) (next-line (- count 1))
                            (let ((to (point))) (goto-char from) (list to nil t)))
                     (vim--motion motion-key count argument)))))
    (when motion
      (let ((range (vim--range from motion)))
        ;; dw at the end of a line does not eat the next line's indentation.
        (when (and (member key '("w" "W")) (= count 1) (> (nth 1 range) (vim--eol from))
                   (> (vim--eol from) from))
          (setq range (list from (vim--eol from) 'char)))
        (vim--apply-range operation range (list 'operator operation key count argument))))
    (vim--reset)))
(defun vim--start-operator (key)
  "Start the operator KEY, or apply it to the Visual selection at once."
  (if (vim--visual-p)
      (if (eq (vim--state) 'visual-block)
          (vim--block-operation key)
          (let ((range (vim--visual-range)))
            (vim--apply-range key range (list 'range key (- (nth 1 range) (car range)) (nth 2 range)))
            (vim--reset)))
      (progn (buffer-put 'vim-operator key)
             (buffer-put 'vim-op-count (vim--number)) (buffer-put 'vim-count nil)
             (set-echo-message key))))

(defun vim--block-spans ()
  "The span on each line of the block selection, as (START END), top first."
  (let* ((at (point)) (anchor (buffer-get 'vim-anchor))
         (left (vim--min (- at (vim--bol at)) (- anchor (vim--bol anchor))))
         (right (+ 1 (vim--max (- at (vim--bol at)) (- anchor (vim--bol anchor)))))
         (start (vim--bol (vim--min at anchor))) (end (vim--bol (vim--max at anchor))) (spans nil))
    (while (<= start end)
      (setq spans (cons (list (vim--min (+ start left) (vim--eol start))
                             (vim--min (+ start right) (vim--eol start))) spans))
      (setq start (+ (vim--eol start) 1)))
    (reverse spans)))
(defun vim--block-sized-spans (width height)
  "Spans WIDTH wide on HEIGHT lines from point down: where . repeats a block
operation."
  (let* ((saved (point)) (column (- saved (vim--bol saved)))
         (start (vim--bol saved)) (spans nil))
    (dotimes (i height)
      (when (<= start (point-max))
        (setq spans (cons (list (vim--min (+ start column) (vim--eol start))
                               (vim--min (+ start column width) (vim--eol start))) spans))
        (setq start (+ (vim--eol start) 1))))
    (reverse spans)))
(defun vim--replicate-block-insert (inserted)
  "Type INSERTED on the other rows of the block a change began with."
  (let ((block (buffer-get 'vim-block-insert)) (saved (point)))
    (when (and block (not (string-match-p "\n" inserted)))
      (dotimes (i (- (car block) 1))
        (goto-line (+ (nth 2 block) i 1)) (end-of-line)
        (let ((width (- (point) (vim--bol (point)))))
          (when (< width (nth 1 block)) (insert (make-string (- (nth 1 block) width) " "))))
        (goto-char (+ (vim--bol (point)) (nth 1 block)))
        (insert inserted)))
    (goto-char saved) (buffer-put 'vim-block-insert nil)))
(defun vim--block-operation (key &optional given-spans)
  "Apply d, c or y, KEY, to the block selection, or to GIVEN-SPANS."
  (when (and (member key '("d" "c" "y"))
             (or (string= key "y") (not (buffer-read-only-p))))
    (let ((spans (or given-spans (vim--block-spans))) (text "") (first t))
      (dolist (span spans)
        (setq text (concat text (if first "" "\n") (buffer-substring (car span) (nth 1 span))))
        (setq first nil))
      (vim--save-text text 'block key)
      (when (member key '("d" "c"))
        (dolist (span (reverse spans)) (delete-region (car span) (nth 1 span))))
      (goto-char (car (car spans)))
      (vim--set-state 'normal) (deactivate-mark) (remove-overlays 'vim-selection) (vim--reset)
      (let ((action (list 'block key (- (nth 1 (car spans)) (car (car spans))) (length spans))))
        (when (not (string= key "y")) (vim--remember action))
        (if (string= key "c")
            (progn
              (vim--begin-insert action)
              (buffer-put 'vim-insert-join t)
              (buffer-put 'vim-block-insert (list (length spans) (- (point) (vim--bol (point))) (line-number-at-point))))
            (vim--clamp))))))
(defun vim--split-lines (text)
  "TEXT split into its lines, without their newlines."
  (let ((start 0) (rows nil))
    (dotimes (i (length text))
      (when (string= (substring text i (+ i 1)) "\n")
        (setq rows (cons (substring text start i) rows) start (+ i 1))))
    (reverse (cons (substring text start) rows))))
(defun vim--paste (key count)
  "Put the register's text COUNT times, after point for p and before it for
P, KEY, in its shape."
  (let* ((name (or (buffer-get 'vim-register) "\""))
         (value (vim--register-value name)) (at (point)))
    (when (and value (not (buffer-read-only-p)))
      (let ((text (car value)) (type (nth 1 value)) (start nil))
        (when (vim--visual-p)
          (let ((range (vim--visual-range)))
            (delete-region (car range) (nth 1 range)) (goto-char (car range))
            (vim--set-state 'normal) (deactivate-mark) (remove-overlays 'vim-selection) (setq key "P")))
        (cond
          ((eq type 'block)
           (let ((column (+ (- (point) (vim--bol (point))) (if (string= key "p") 1 0)))
                 (rows (vim--split-lines text)) (line (line-number-at-point)))
             (setq start (+ (vim--bol (point)) column))
             (dolist (row rows)
               (goto-line line) (end-of-line)
               (let ((width (- (point) (vim--bol (point)))))
                 (when (< width column) (insert (make-string (- column width) " "))))
               (goto-char (+ (vim--bol (point)) column))
               (dotimes (i count) (insert row))
               (when (cdr (member row rows))
                 (end-of-line) (when (= (point) (point-max)) (insert "\n")))
               (setq line (+ line 1)))
             (goto-char start)))
          ((eq type 'line)
           (if (string= key "P") (goto-char (vim--bol (point)))
               (goto-char (vim--line-end (point))))
           (when (and (= (point) (point-max)) (> (point) 0) (not (string= (vim--char (- (point) 1)) "\n")))
             (insert "\n"))
           (setq start (point))
           (dotimes (i count)
             (insert text)
             (when (and (> (length text) 0) (not (string= (substring text -1) "\n"))) (insert "\n")))
           (goto-char (vim--first start)))
          (t
           (when (string= key "p") (goto-char (vim--min (vim--eol (point)) (+ (point) 1))))
           (setq start (point))
           (dotimes (i count) (insert text))
           (goto-char (vim--max start (- (point) 1)))))
        (vim--remember (list 'paste key count name))
        (vim--clamp)))))

;;; Search uses the editor's regex scanner and character offsets.
(defun vim--search-next (direction count)
  "Go to the COUNTth match of the last search, forward when DIRECTION is
positive; nil when there is none."
  (when (and vim-search-pattern (not (string= vim-search-pattern "")))
    (let* ((scan (scan-buffer vim-search-pattern vim-search-regexp 1000000))
           (matches (nth 1 scan)) (original (point)) (found nil))
      (dotimes (i count)
        (setq found nil)
        (dolist (entry (if (> direction 0) matches (reverse matches)))
          (when (and (not found) (if (> direction 0) (> (nth 4 entry) (point)) (< (nth 4 entry) (point))))
            (setq found (nth 4 entry))))
        (when (and (not found) vim-search-wrap matches)
          (setq found (nth 4 (if (> direction 0) (car matches) (car (reverse matches))))))
        (when found (goto-char found)))
      (when (not found) (goto-char original) (set-echo-message "Pattern not found"))
      found)))
(defun vim--search-confirm (input)
  "Search for INPUT, or for the last pattern when it is empty: what / and ?
do with their answer."
  (when (not (string= input "")) (setq vim-search-pattern input vim-search-regexp t))
  (let ((from (point)) (operation (buffer-get 'vim-operator))
        (count (* (vim--number) (or (buffer-get 'vim-op-count) 1))))
    (condition-case error
        (when (vim--search-next vim-search-direction count)
          (when operation
            (let ((motion (list (point) nil nil)))
              (goto-char from)
              (vim--apply-range operation (vim--range from motion)
                                (list 'search operation vim-search-direction count)))))
      (error (set-echo-message "Invalid search pattern")))
    (vim--reset) (vim--clamp) (vim--show-visual)))
(defun vim--search-cancel ()
  "Forget the command a search prompt was part of, as the prompt closes."
  (vim--reset))
(defun vim--search-prompt (key)
  "Ask for a pattern, to search forward for / and backward for ?, KEY."
  (setq vim-search-direction (if (string= key "/") 1 -1))
  (minibuffer-read key 'vim--search-confirm nil 'vim--search-cancel))
(defun vim--search-word (key count)
  "Search for the word at point, forward for * and backward for #, KEY."
  (let ((range (vim--object "w" nil 1)))
    (when range
      (setq vim-search-pattern (buffer-substring (car range) (nth 1 range))
            vim-search-regexp t vim-search-direction (if (string= key "*") 1 -1))
      (setq vim-search-pattern
            (if (= (vim--class (car range) nil) 1)
                (concat "\\b" (vim--regexp-quote vim-search-pattern) "\\b")
                (vim--regexp-quote vim-search-pattern)))
      (vim--search-next vim-search-direction count))))

(defun vim--join (count)
  "Join COUNT lines, two at least, dropping the indentation of each joined
one."
  (when (not (buffer-read-only-p))
    (dotimes (i (vim--max 1 (- count 1)))
      (let ((end (vim--eol (point))))
        (when (< end (point-max))
          (let ((next (vim--first (+ end 1))))
            (delete-region end next) (goto-char end)
            (when (and (> end (vim--bol end)) (not (member (vim--char (- end 1)) '(" " "\t"))))
              (insert " ") (goto-char end))))))
    (vim--remember (list 'join count))))
(defun vim--replace (c count)
  "Replace COUNT characters at point with C, for r."
  (let ((at (point)))
    (when (and (not (buffer-read-only-p)) (<= (+ at count) (vim--eol at)))
      (delete-region at (+ at count)) (goto-char at)
      (dotimes (i count) (insert (if (string= c "<ret>") "\n" c)))
      (goto-char (vim--max at (- (point) 1)))
      (vim--remember (list 'replace c count)))))
(defun vim--repeat (count explicit)
  "Repeat the last edit, for .; EXPLICIT when COUNT was typed."
  (when vim-last-change
    (let* ((saved vim-last-change) (action (car saved)) (diff (nth 1 saved))
           (kind (car action)) (was-replaying vim--replaying))
      (setq vim--replaying t)
      (unwind-protect
        (progn
      (cond
        ((member kind '(insert open)) (vim--insert-command (nth 1 action) (if explicit count (nth 2 action))))
        ((eq kind 'operator)
         (buffer-put 'vim-operator (nth 1 action))
         (vim--operator-motion (nth 2 action) (if explicit count (nth 3 action)) (nth 4 action)))
        ((eq kind 'range)
         (vim--apply-range (nth 1 action)
                           (if (eq (nth 3 action) 'line)
                               (vim--line-range (point) (point))
                               (list (point) (vim--min (point-max) (+ (point) (nth 2 action))) 'char)) action))
        ((eq kind 'block)
         (vim--block-operation (nth 1 action) (vim--block-sized-spans (nth 2 action) (nth 3 action))))
        ((eq kind 'replace-mode) (vim--replace-mode (nth 1 action)))
        ((eq kind 'object)
         (vim--apply-range (nth 1 action) (vim--object (nth 2 action) (nth 3 action) (if explicit count (nth 4 action))) action))
        ((eq kind 'shortcut)
         (buffer-put 'vim-count (if explicit count (nth 2 action)))
         (vim--dispatch (nth 1 action)))
        ((eq kind 'paste)
         (buffer-put 'vim-register (nth 3 action)) (vim--paste (nth 1 action) (if explicit count (nth 2 action))))
        ((eq kind 'replace) (vim--replace (nth 1 action) (if explicit count (nth 2 action))))
        ((eq kind 'join) (vim--join (if explicit count (nth 1 action))))
        ((eq kind 'search)
         (let ((from (point)))
           (when (vim--search-next (nth 2 action) (if explicit count (nth 3 action)))
             (let ((to (point))) (goto-char from)
               (vim--apply-range (nth 1 action) (vim--range from (list to nil nil)) action))))))
      (when (and diff (member (vim--state) '(insert replace)))
        (let ((at (vim--max 0 (+ (point) (car diff)))))
          (delete-region at (vim--min (point-max) (+ at (nth 1 diff))))
          (goto-char at) (insert (nth 2 diff))
          (when (member kind '(insert open))
            (dotimes (i (- (if explicit count (nth 2 action)) 1))
              (when (eq kind 'open) (insert "\n" (make-string (current-indentation) " ")))
              (insert (nth 2 diff)))))
        (when (buffer-get 'vim-block-insert) (vim--replicate-block-insert (nth 2 diff)))
        ;; This replay already applied the counted insertion.
        (buffer-put 'vim-insert-before nil)
        (vim-normal-state))
      (vim--reset) (vim--clamp))
        (setq vim--replaying was-replaying vim-last-change saved)))))

(defun vim--mark-set (key)
  "Set the mark KEY at point: a lowercase letter for this buffer, an
uppercase one with the buffer."
  (when (string-match-p "^[a-zA-Z]$" key)
    (if (string-match-p "^[A-Z]$" key)
        (put 'vim-marks (intern key) (list (current-buffer) (point)))
        (buffer-put (concat "vim-mark-" key) (point)))))
(defun vim--mark-jump (key exact)
  "Go to the mark KEY: to its character with EXACT, else to the first
nonblank of its line; nil when it is not set."
  (let ((target (buffer-get (concat "vim-mark-" key))))
    (when (string-match-p "^[A-Z]$" key)
      (let ((saved (get 'vim-marks (intern key))))
        (when saved (switch-to-buffer (car saved)) (setq target (nth 1 saved)))))
    (when target (goto-char target) (when (not exact) (goto-char (vim--first (point)))) t)))
(defun vim--macro-start (register)
  "Start recording keys into REGISTER, a letter."
  (when (and (string-match-p "^[a-zA-Z]$" register) (not (kmacro-recording-p)))
    (setq vim-recording-register register)
    (kmacro-start-macro)))
(defun vim--macro-stop ()
  "Stop recording, and store the keys in the register they were for."
  (kmacro-end-macro)
  (let ((keys (kbd-macro-keys)) (name vim-recording-register))
    ;; q is dispatched through vim--key, so the recorder sees its terminating key.
    (when keys
      (setq keys (if (string= keys "q") "" (substring keys 0 (- (length keys) 2))))
      (when (string-match-p "^[A-Z]$" name)
        (setq name (downcase name))
        (let ((old (vim--register-value name)))
          (when old (setq keys (concat (car old) " " keys)))))
      (vim--put-register name (list keys 'macro))))
  (setq vim-recording-register nil))
(defun vim--macro-call (key count)
  "Play the macro in register KEY, or the one played last for @, COUNT times."
  (let* ((name (if (string= key "@") vim-last-macro-register key))
         (value (when name (vim--register-value name))))
    (when (and value (eq (nth 1 value) 'macro))
      (setq vim-last-macro-register name)
      (kmacro-call-keys (car value) count))))

;;; Ex commands deliberately use existing save/quit safety and prompts.
(defun vim--ex-confirm (input)
  "Run the Ex command INPUT: what : does with its answer."
  (let ((command input))
    (cond
      ((or (string-match-p "^s[^[:alnum:] ]" command) (string-prefix-p "%s" command)
           (string-prefix-p "'<,'>s" command) (string-match-p "^[0-9]+(,[0-9]+)?s" command))
       (condition-case error (vim--substitute command) (error (message "Vim: %s" error))))
      ((member command '("w" "write")) (save-buffer))
      ((member command '("q" "quit")) (quit))
      ((member command '("wq" "x")) (save-buffer) (when (not (buffer-modified-p)) (quit)))
      ((string= command "q!") (quit-without-saving))
      ((string= command "w!") (save-buffer))
      ((string= command "e") (call-interactively 'find-file))
      ((string-prefix-p "e " command) (find-file (substring command 2)))
      ((string-prefix-p "w " command) (write-file (substring command 2)))
      ((string-prefix-p "b " command) (switch-to-buffer (substring command 2)))
      ((member command '("bd" "bdelete")) (call-interactively 'kill-buffer))
      ((string= command "split") (split-window-below))
      ((string= command "vsplit") (split-window-right))
      ((string= command "only") (delete-other-windows))
      ((string= command "close") (delete-window))
      ((string= command "set number") (setq display-line-numbers t))
      ((string= command "set relativenumber") (setq display-line-numbers 'relative))
      ((string= command "set nonumber") (setq display-line-numbers nil))
      ((string= command "set ignorecase") (setq case-fold-search t))
      ((string= command "set noignorecase") (setq case-fold-search nil))
      ((string= command "set nowrapscan") (setq vim-search-wrap nil))
      ((string= command "set wrapscan") (setq vim-search-wrap t))
      ((member command '("help" "h"))
       (if (functionp 'manpage)
           (manpage "vim")
           (set-echo-message "No manual reader: the manpage module is not loaded")))
      ((string= command "noh") (deactivate-mark))
      ((string-match-p "^[0-9]+$" command) (goto-line (string-to-number command)) (vim--clamp))
      (t (set-echo-message (concat "Unsupported Ex command: " command))))))
(defun vim--ex ()
  "Ask for an Ex command, leaving Visual state."
  (when (vim--visual-p)
    (vim--set-state 'normal) (deactivate-mark) (remove-overlays 'vim-selection))
  (minibuffer-read ":" 'vim--ex-confirm nil nil))

(defun vim--pending-key (pending key)
  "Finish with KEY the command PENDING, which waits for one more key."
  (buffer-put 'vim-pending nil)
  (let ((count (* (vim--number) (or (buffer-get 'vim-op-count) 1))))
    (cond
      ((string= pending "\"") (buffer-put 'vim-register key))
      ((string= pending "m") (vim--mark-set key) (vim--reset))
      ((member pending '("'" "`"))
       (let ((from (point)) (operation (buffer-get 'vim-operator)))
         (when (vim--mark-jump key (string= pending "`"))
           (when operation
             (let ((to (point))) (goto-char from)
               (vim--apply-range operation (vim--range from (list to nil (string= pending "'")))
                                 (list 'range operation (abs (- to from)) (if (string= pending "'") 'line 'char)))))))
       (vim--reset))
      ((string= pending "q") (vim--macro-start key) (vim--reset))
      ;; The macro plays at once, so the count is cleared first: else its
      ;; first command would take it as well.
      ((string= pending "@") (vim--reset) (vim--macro-call key count))
      ((string= pending "r") (vim--replace key count) (vim--reset))
      ((member pending '("f" "F" "t" "T"))
       (setq vim-last-find (list pending key))
       (if (buffer-get 'vim-operator) (vim--operator-motion pending count key)
           (let ((motion (vim--motion pending count key)))
             (when motion (goto-char (car motion)) (vim--clamp)))
           )
       (vim--reset))
      ((member pending '("i" "a"))
       (let ((range (vim--object key (string= pending "a") count))
             (operation (buffer-get 'vim-operator)))
         (when range
           (if operation
               (vim--apply-range operation range (list 'object operation key (string= pending "a") count))
               (progn
                 (buffer-put 'vim-anchor (car range))
                 (goto-char (vim--max (car range) (- (nth 1 range) 1)))
                 (vim--show-visual)))))
       (vim--reset))
      ((string= pending "g")
       (cond
         ((member key '("g" "e"))
          (if (buffer-get 'vim-operator) (vim--operator-motion (concat "g" key) count)
              (let ((motion (vim--motion (concat "g" key) count)))
                (when motion (goto-char (car motion)) (vim--clamp))
                (vim--reset))))
         ((member key '("U" "u" "~")) (vim--start-operator (concat "g" key)))
         ((string= key "v")
          (let ((selection (buffer-get 'vim-last-visual)))
            (when selection (buffer-put 'vim-anchor (car selection)) (goto-char (nth 1 selection))
                  (vim--set-state (nth 2 selection)) (vim--show-visual))))
         (t (vim--reset))))
      ((string= pending "z") (recenter) (vim--reset))
      ((string= pending "Z")
       (cond ((string= key "Z") (save-buffer) (when (not (buffer-modified-p)) (quit)))
             ((string= key "Q") (quit-without-saving))) (vim--reset))
      ((string= pending "C-w")
       (cond ((member key '("s" "C-s")) (split-window-below))
             ((member key '("v" "C-v")) (split-window-right))
             ((member key '("w" "C-w" "h" "j" "k" "l")) (other-window))
             ((string= key "o") (delete-other-windows))
             ((string= key "c") (delete-window))) (vim--reset)))))

(defun vim--key (key)
  "Run the Normal or Visual state key KEY, reporting an error in the echo
area."
  (condition-case error
      (vim--dispatch key)
    (error (vim--reset) (message "Vim: %s" error))))
(defun vim--dispatch (key)
  "Run the Normal or Visual state key KEY."
  (when (string= key " ") (setq key "<space>"))
  (when (vim--visual-p)
    (buffer-put 'vim-last-visual (list (buffer-get 'vim-anchor) (point) (vim--state))))
  (cond
    ((member key '("<esc>" "C-[" "C-g" "C-c")) (vim-normal-state))
    ((buffer-get 'vim-pending) (vim--pending-key (buffer-get 'vim-pending) key))
    ((and (string-match-p "^[0-9]$" key) (or (not (string= key "0")) (buffer-get 'vim-count))) (vim--digit key))
    ((and (or (buffer-get 'vim-operator) (vim--visual-p)) (member key '("i" "a")))
     (buffer-put 'vim-pending key))
    ((member key '("g" "z" "Z" "f" "F" "t" "T" "m" "'" "`" "\"" "@" "r" "C-w"))
     (buffer-put 'vim-pending key))
    ((member key '("/" "?")) (vim--search-prompt key))
    ((buffer-get 'vim-operator)
     (vim--operator-motion key (* (vim--number) (or (buffer-get 'vim-op-count) 1))))
    (t
      (let ((count (vim--number)) (explicit (buffer-get 'vim-count)) (motion nil))
        (cond
          ((member key '("d" "c" "y" ">" "<" "=")) (vim--start-operator key))
          ((string= key "v") (vim--visual 'visual) (vim--reset))
          ((string= key "V") (vim--visual 'visual-line) (vim--reset))
          ((string= key "C-v") (vim--visual 'visual-block) (vim--reset))
          ((and (vim--visual-p) (string= key "o"))
           (let ((anchor (buffer-get 'vim-anchor))) (buffer-put 'vim-anchor (point)) (goto-char anchor)))
          ((string= key "R") (vim--replace-mode count) (vim--reset))
          ((member key '("i" "a" "I" "A" "o" "O")) (vim--insert-command key count) (vim--reset))
          ((member key '("x" "s" "X" "D" "C" "S" "Y"))
           (let* ((at (point)) (operation (cond ((member key '("s" "C" "S")) "c") ((string= key "Y") "y") (t "d")))
                  (range (cond ((vim--visual-p) (vim--visual-range))
                               ((member key '("S" "Y"))
                                (goto-char (vim--bol at)) (next-line (- count 1))
                                (let ((to (point))) (goto-char at) (vim--line-range at to)))
                               ((member key '("D" "C")) (list at (vim--eol at) 'char))
                               ((string= key "X") (list (vim--max (vim--bol at) (- at count)) at 'char))
                               (t (list at (vim--min (vim--eol at) (+ at count)) 'char)))))
             (vim--apply-range operation range (if (vim--visual-p)
                                   (list 'range operation (- (nth 1 range) (car range)) (nth 2 range))
                                   (list 'shortcut key count))))
           (vim--reset))
          ((member key '("p" "P")) (vim--paste key count) (vim--reset))
          ((and (vim--visual-p) (member key '("u" "U")))
           (vim--start-operator (if (string= key "u") "gu" "gU")))
          ((string= key "u") (dotimes (i count) (undo)) (vim--clamp) (vim--reset))
          ((string= key "C-r") (dotimes (i count) (redo)) (vim--clamp) (vim--reset))
          ((string= key ".") (vim--repeat count explicit))
          ((string= key "J") (vim--join count) (vim--reset))
          ((string= key "~")
           (let ((range (if (vim--visual-p) (vim--visual-range)
                           (list (point) (vim--min (vim--eol (point)) (+ (point) count)) 'char))))
             (vim--apply-range "~" range (list 'range "~" (- (nth 1 range) (car range)) 'char)))
           (vim--reset))
          ((member key '("n" "N"))
           (vim--search-next (* vim-search-direction (if (string= key "n") 1 -1)) count)
           (vim--clamp) (vim--reset))
          ((member key '("*" "#")) (vim--search-word key count) (vim--clamp) (vim--reset))
          ((string= key ":") (vim--ex) (vim--reset))
          ((string= key "q")
           (if vim-recording-register (vim--macro-stop) (buffer-put 'vim-pending "q")))
          ((member key '("C-d" "C-u" "C-f" "C-b"))
           (next-line (* count (if (member key '("C-u" "C-b")) -1 1)
                         (if (member key '("C-f" "C-b")) (vim--max 1 (- frame-height 2))
                             (vim--max 1 (floor (/ frame-height 2))))))
           (vim--clamp) (vim--reset))
          ((string= key "C-e") (scroll-up-command count) (vim--reset))
          ((string= key "C-y") (scroll-down-command count) (vim--reset))
          ((string= key "K") (when (functionp 'manpage-at-point) (manpage-at-point)) (vim--reset))
          (t (setq motion (vim--motion key count))
             (when motion (goto-char (car motion)) (vim--clamp) (vim--show-visual))
             (vim--reset)))))))

;;; Replace sessions use the same diff and undo contract as Insert.
(defun vim--replace-mode (count)
  "Enter Replace state, for R."
  (when (not (buffer-read-only-p))
    (vim--begin-insert (list 'replace-mode count))
    (buffer-put 'vim-replace-stack nil)
    (vim--set-state 'replace)))
(defun vim--replace-key (key)
  "Type KEY in Replace state: overwrite a character, give one back for
Backspace, or leave."
  (cond
    ((member key '("<esc>" "C-[" "C-c")) (vim-normal-state))
    ((member key '("<backspace>" "C-h"))
     (let ((stack (buffer-get 'vim-replace-stack)))
       (when stack
         (let* ((item (car stack)) (at (car item)))
           (delete-region at (+ at (nth 2 item))) (goto-char at) (insert (nth 1 item))
           (goto-char at) (buffer-put 'vim-replace-stack (cdr stack))))))
    ((member key '("<left>" "<right>" "<up>" "<down>"))
     (buffer-put 'vim-replace-stack nil)
     (cond ((string= key "<left>") (backward-char)) ((string= key "<right>") (forward-char))
           ((string= key "<up>") (previous-line)) (t (next-line))))
    (t
     (let* ((at (point)) (text (if (string= key "<ret>") "\n" key))
            (old (if (and (< at (vim--eol at)) (not (string= text "\n"))) (vim--char at) "")))
       (when (not (string= old "")) (delete-region at (+ at 1)))
       (goto-char at) (insert text)
       (buffer-put 'vim-replace-stack (cons (list at old (length text)) (buffer-get 'vim-replace-stack)))))))

(defun vim--regexp-quote (text)
  "TEXT with every character a regular expression gives a meaning to escaped."
  (let ((answer ""))
    (dotimes (i (length text))
      (let ((c (substring text i (+ i 1))))
        (setq answer (concat answer (if (member c '("\\" "." "+" "*" "?" "(" ")" "[" "]" "{" "}" "^" "$" "|")) "\\" "") c))))
    answer))
(defun vim--ex-field (text start delimiter &optional replacement)
  "The field of TEXT from START to the next DELIMITER, as (FIELD NEXT), or
nil when no DELIMITER ends it; with REPLACEMENT, the rest of TEXT then."
  (let ((at start) (field "") (finished nil))
    (while (and (< at (length text)) (not finished))
      (let ((c (substring text at (+ at 1))))
        (cond ((string= c delimiter) (setq finished t))
              ((and (string= c "\\") (< (+ at 1) (length text)))
               (setq at (+ at 1))
               (let ((next (substring text at (+ at 1))))
                 (setq field (concat field (if (string= next delimiter) "" "\\") next))))
              (t (setq field (concat field c)))))
      (setq at (+ at 1)))
    (if (or finished replacement) (list field at) nil)))
(defun vim--expand-replacement (replacement subject)
  "REPLACEMENT with &, \\1 to \\9, \\t, \\n and \\r expanded from the last
match in SUBJECT."
  (let ((at 0) (answer ""))
    (while (< at (length replacement))
      (let ((c (substring replacement at (+ at 1))))
        (cond ((string= c "&") (setq answer (concat answer (or (match-string 0 subject) ""))))
              ((and (string= c "\\") (< (+ at 1) (length replacement)))
               (setq at (+ at 1))
               (let ((next (substring replacement at (+ at 1))))
                 (setq answer (concat answer
                   (cond ((string-match-p "^[0-9]$" next) (or (match-string (string-to-number next) subject) ""))
                         ((member next '("n" "r")) "\n") ((string= next "t") "\t") (t next))))))
              (t (setq answer (concat answer c)))))
      (setq at (+ at 1)))
    answer))
(defun vim--substitute (input)
  "Run the substitution INPUT -- :s, :%s, :N,Ms or :'<,'>s -- on the lines it
names."
  (let ((at 0) (start (vim--bol (point))) (end (vim--line-end (point))) (range nil))
    (cond
      ((string-prefix-p "%s" input) (setq at 1 start 0 end (point-max)))
      ((string-prefix-p "'<,'>s" input)
       (setq at 5 range (buffer-get 'vim-last-visual))
       (when range (setq start (vim--bol (car range)) end (vim--line-end (nth 1 range)))))
      ((string-match "^([0-9]+)(,([0-9]+))?s" input)
       (let* ((first (string-to-number (match-string 1 input)))
              (last (if (match-string 3 input) (string-to-number (match-string 3 input)) first))
              (saved (point)) (next (- (match-end 0) 1)))
         (goto-line first) (setq start (vim--bol (point)))
         (goto-line last) (setq end (vim--line-end (point)))
         (goto-char saved) (setq at next))))
    (when (and (< (+ at 1) (length input)) (string= (substring input at (+ at 1)) "s"))
      (let* ((delimiter (substring input (+ at 1) (+ at 2)))
             (pattern-field (vim--ex-field input (+ at 2) delimiter)))
        (when (not pattern-field) (error "Missing search delimiter"))
        (let* ((replacement-field (vim--ex-field input (nth 1 pattern-field) delimiter t))
               (pattern (if (string= (car pattern-field) "") vim-search-pattern (car pattern-field)))
               (replacement (car replacement-field))
               (flags (substring input (nth 1 replacement-field)))
               (saved-case case-fold-search) (changes nil) (last-line nil))
          (when (not pattern) (error "No previous search pattern"))
          (when (not (string-match-p "^[giI]*$" flags)) (error "Supported substitution flags: g, i, I"))
          (when (buffer-read-only-p) (error "Buffer is read-only"))
          (setq vim-search-pattern pattern vim-search-regexp t)
          (unwind-protect
              (progn
                (when (string-match-p "i" flags) (setq case-fold-search t))
                (when (string-match-p "I" flags) (setq case-fold-search nil))
                (let ((scan (scan-buffer pattern t 1000000)))
                  (when (car scan) (error "Too many matches"))
                  (dolist (entry (nth 1 scan))
                    (let ((from (nth 4 entry)) (to (+ (nth 4 entry) (- (nth 7 entry) (nth 6 entry)))))
                      (when (and (>= from start) (< from end) (<= to end)
                                 (or (string-match-p "g" flags) (not (equal last-line (nth 2 entry)))))
                        ;; Match data belongs to this match until expansion finishes.
                        (string-match (if case-fold-search (concat "(?i)" pattern) pattern)
                                      (nth 5 entry) (nth 6 entry))
                        (setq changes (cons (list from to (vim--expand-replacement replacement (nth 5 entry))) changes))
                        (setq last-line (nth 2 entry)))))))
            (setq case-fold-search saved-case))
          (dolist (change changes)
            (delete-region (car change) (nth 1 change)) (goto-char (car change)) (insert (nth 2 change)))
          (vim--clamp)
          (message "%s substitutions" (length changes))
          t)))))

(make-mode 'vim-normal-map)
(make-mode 'vim-insert-map)
(make-mode 'vim-replace-map)
(define-key 'vim-normal-map "<char>" 'vim--key)
(define-key 'vim-insert-map "<char>" 'self-insert)
(define-key 'vim-replace-map "<char>" 'vim--replace-key)
;; Bind every printable ASCII key so unsupported Normal keys cannot insert text.
(let ((keys " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"))
  (dotimes (i (length keys))
    (let* ((key (substring keys i (+ i 1))) (spelling (if (string= key " ") "<space>" key)))
      (define-key 'vim-replace-map spelling (list 'vim--replace-key key))
      (define-key 'vim-normal-map spelling (list 'vim--key key)))))
(dolist (key '("<esc>" "<ret>" "<backspace>" "<left>" "<right>" "<up>" "<down>"
               "C-[" "C-g" "C-c" "C-r" "C-v" "C-w" "C-d" "C-u" "C-f" "C-b" "C-e" "C-y"))
  (define-key 'vim-normal-map key (list 'vim--key key)))
(dolist (key '("<esc>" "C-[" "C-c" "<backspace>" "C-h" "<ret>" "<left>" "<right>" "<up>" "<down>"))
  (define-key 'vim-replace-map key (list 'vim--replace-key key)))
(dolist (key '("<esc>" "C-[" "C-c")) (define-key 'vim-insert-map key 'vim-normal-state))
(define-key 'vim-insert-map "C-w" 'backward-kill-word)
(define-key 'vim-insert-map "C-h" 'delete-backward-char)
(define-key 'vim-insert-map "C-u" '(delete-region (vim--bol (point)) (point)))
(add-hook nil "pre-command-hook" 'vim--pre-command)
(add-hook nil "post-command-hook" 'vim--post-command)
(add-hook nil "after-change-major-mode-hook" 'vim--apply-policy)
(add-hook nil "after-set-visited-file-name-hook" 'vim--apply-policy)
(log "End of vim.lisp")
