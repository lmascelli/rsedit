;;; help --- what the editor can tell you about itself.
;;;
;;; `C-h f' a function, `C-h v' a variable, `C-h k' a key, `C-h b' every
;;; binding in effect, `C-h a' anything whose name matches, `C-h ?' this list.
;;; Half-way through a prefix, `C-h' says what the prefix goes on to.
;;;
;;; # What is in here and what is not
;;;
;;; Nothing here knows what a binding *is*. `key-binding', `where-is',
;;; `keymap-bindings', `function-doc', `variable-doc' and the rest are built
;;; in, and they answer out of the same keymaps and the same environment the
;;; editor runs on. A help command that a missing module could quietly make
;;; wrong would be worse than no help command at all, because it is believed.
;;;
;;; What is here is the *page*: which facts go together, in what order, and in
;;; which buffer. Replace it and the answers do not move.
;;;
;;; # Every command comes in two halves
;;;
;;; `describe-function' prompts; `help-function' takes a name. The prompting
;;; half is what a key is bound to, the other is what anything else calls --
;;; a script, a test, another page's cross-reference. Splitting them is what
;;; lets the prompt use completion over `all-functions' rather than the plain
;;; string an argument spec would read.
;;;
;;; # The back stack
;;;
;;; A help page is mostly links: a function's page names the variables it
;;; reads, a key's page names the command it runs. Following one and not being
;;; able to get back is the difference between a help system and a maze, so
;;; every page that is opened remembers the one before it, and `l' returns.
;;; The stack holds *descriptions* -- ("function" "insert") -- rather than the
;;; text, so a page revisited is rebuilt from the editor as it is now rather
;;; than replayed from how it was.

(make-mode 'help-mode)

(defconst help-buffer-name "*Help*"
  "The one buffer help pages are shown in.

One rather than one per page, for the reason `man' shows one manual at a time:
a page is something you consult and leave, and accumulating them would leave a
buffer list nobody wants to read.")

(defvar help-key "C-h"
  "The key that opens help, and -- its last key -- what asks a half-typed
prefix where it goes

`C-h' is Emacs' answer and the one most fingers already know. It is a variable
because it cannot be the right answer everywhere: a terminal that still sends
Backspace as `C-h' makes this prefix and that key the same keystroke. On such a
terminal, put

  (setq help-key \"C-c h\")
  (install-help-keys)

in your init.lisp and the whole tree moves. Prefix help moves with it and asks
on the *last* key of the sequence -- plain `h' after `C-x', in that example --
since half-way through a sequence there is only ever one key to press. It is
asked about sequences that are bound to nothing, so it takes nothing away.")

(defvar help--history nil
  "Pages visited before this one, most recent first.

Each entry is what it takes to build a page again -- (\"function\" \"insert\")
-- and not the text of it, so `help-back' shows what is true now rather than
what was true when the page was first drawn.")

(defvar help--current nil
  "The page being shown, in the same form as an entry of `help--history'.")

(defconst help-history-length 50
  "How many pages back `l' can walk. Older ones are forgotten.")

;; ---------------------------------------------------------------------------
;; Building a page
;; ---------------------------------------------------------------------------

(defun help--pad (text width)
  "TEXT with spaces added until it is WIDTH wide. Longer text is left alone."
  (let ((out text))
    (while (< (length out) width)
      (setq out (concat out " ")))
    out))

(defun help--lines (text)
  "TEXT split into its lines."
  (split-string text "\n"))

(defun help--first-line (text)
  "The first line of TEXT, or an empty string when there is none."
  (if (or (null text) (string= text ""))
      ""
      (car (help--lines text))))

(defun help--join (lines)
  "LINES as one string, one per line."
  (let ((out ""))
    (mapc (lambda (line)
            (setq out (if (string= out "") line (concat out "\n" line))))
          lines)
    out))

(defun help--last-key (sequence)
  "The last key of SEQUENCE, which is the one pressed half-way through one.

The editor asks the same question of `help-key' in Rust; this is here so the
page shows what will actually work rather than the whole prefix."
  (let ((parts (split-string sequence " ")))
    (nth (- (length parts) 1) parts)))

(defun help--symbol (name)
  "NAME as a symbol, whether it arrived as one or as a string."
  (if (stringp name) (intern name) name))

(defun help--name (thing)
  "THING as a string, whether it arrived as a symbol, a string, or a form."
  (cond ((null thing) "nil")
        ((stringp thing) thing)
        ((symbolp thing) (symbol-name thing))
        ((listp thing) (help--name (car thing)))
        (t (format "%s" thing))))

;; ---------------------------------------------------------------------------
;; The pages themselves
;;
;; Each returns a list of lines. None of them touches a buffer, which is what
;; makes them testable without one and replaceable without touching what they
;; are made of.
;; ---------------------------------------------------------------------------

(defun help--signature (name)
  "How NAME is called, as a line, or nil when nothing can be said.

A Lisp function has a parameter list to read. A primitive does not -- it is a
Rust function -- but the first line of its documentation is its call, written
there for exactly this reason."
  (let ((arglist (function-arglist (help--symbol name))))
    (if arglist
        (concat "(" name " " (help--join-words arglist) ")")
        (let ((first (help--first-line (function-doc (help--symbol name)))))
          (if (string= (substring first 0 1) "(")
              (car (split-string first ":"))
              nil)))))

(defun help--join-words (words)
  "WORDS joined with single spaces."
  (let ((out ""))
    (mapc (lambda (word)
            (setq out (if (string= out "") word (concat out " " word))))
          words)
    out))

(defun help--function-lines (name)
  "The page for the function NAME."
  (let* ((symbol (help--symbol name))
         (doc (function-doc symbol)))
    (if (null doc)
        (list (concat name " is not a function.")
              ""
              (concat "Nothing is bound to that name. `" help-key
                      " a' searches every name there is."))
        (append
         (list name "")
         (let ((signature (help--signature name)))
           (if signature (list signature "") nil))
         (let ((keys (where-is symbol)))
           (if keys
               (list (concat "Bound to: " (help--join-words keys)) "")
               nil))
         (if (commandp symbol)
             (let ((specs (command-specs name)))
               (list (if specs
                         (concat "A command. It asks for: " (help--join-words specs))
                         "A command. It takes no arguments.")
                     ""))
             nil)
         (help--lines doc)))))

(defun help--variable-lines (name)
  "The page for the variable NAME."
  (let* ((symbol (help--symbol name))
         (doc (variable-doc symbol)))
    (if (null doc)
        (list (concat name " is not a variable.")
              ""
              (concat "It is neither bound nor documented. `" help-key
                      " a' searches every name there is."))
        (append
         (list name
               ""
               ;; Bound and documented are different questions, and a variable
               ;; documented before anything sets it is a real state -- see
               ;; `variable-doc'. Saying "nil" for it would be a lie about a
               ;; value it does not have.
               (if (boundp symbol)
                   (format "Value: %s" (symbol-value symbol))
                   "Not set. Whatever reads it will fail until something does.")
               "")
         (help--lines doc)))))

(defun help--key-lines (keys target source)
  "The page for KEYS, which runs TARGET, from the map named SOURCE.

The three are passed in rather than looked up again because they came from the
editor's own resolution -- see `read-key-sequence'. A page that asked the
keymaps a second time would be answering a slightly different question."
  (append
   (list (concat "Key: " keys) "")
   ;; `null' first: `string=' is asked about a string, and a key bound to
   ;; nothing has no source at all.
   (cond ((null source)
          (list (concat keys " is not bound to anything.")
                ""
                (if (keys-with-prefix keys)
                    (concat "It is a prefix, though -- press "
                            (help--last-key help-key)
                            " after it to see what it continues with.")
                    "Nothing continues it either.")))
         ((string= source "prefix-argument")
          (list (concat keys " is read as a prefix argument.")
                ""
                "It never reaches a keymap: the digits after it build a count"
                "that the next command receives. `C-u 4 C-n' moves down four"
                "lines."))
         (t
          (append
           (list (concat keys " runs " (help--name target)
                         "  (from the " source " keymap)")
                 "")
           (help--function-lines (help--name target)))))))

(defun help--bindings-lines (buffer)
  "Every binding in effect in BUFFER, grouped by the map it comes from.

BUFFER is named rather than assumed, because by the time a page is rebuilt --
going back to it with `l' -- the current buffer is the help buffer itself, and
the bindings in effect *there* are not the ones anybody asked about."
  (let ((transient nil)
        (mode nil)
        (global nil))
    (mapc (lambda (binding)
            (let ((line (concat "  " (help--pad (nth 0 binding) 14)
                                (help--name (nth 1 binding))))
                  (source (nth 2 binding)))
              (cond ((string= source "transient") (setq transient (append transient (list line))))
                    ((string= source "mode") (setq mode (append mode (list line))))
                    (t (setq global (append global (list line)))))))
          (with-current-buffer buffer 'keymap-bindings))
    (let ((mode-name (with-current-buffer buffer (lambda () (help--name (major-mode))))))
      (append
       (list (concat "Bindings in effect in " buffer " (" mode-name ")") "")
       (if transient
           (append (list "Transient -- until the question is answered") transient (list ""))
           nil)
       (if mode (append (list (concat "Mode -- " mode-name)) mode (list "")) nil)
       (if global (append (list "Global -- everywhere") global) nil)))))

(defun help--prefix-lines (prefix)
  "What PREFIX continues with."
  (let ((onwards (keys-with-prefix prefix)))
    (if (null onwards)
        (list (concat prefix " leads nowhere."))
        (append
         (list (concat prefix " continues with:") "")
         (mapcar (lambda (binding)
                   (concat "  " (help--pad (nth 0 binding) 14)
                           (help--name (nth 1 binding))))
                 onwards)))))

(defun help--mode-lines (buffer)
  "The page for the major mode BUFFER is in."
  (let* ((name (with-current-buffer buffer (lambda () (help--name (major-mode)))))
         (doc (variable-doc (help--symbol name))))
    (append
     (list (concat "Mode: " name) "")
     (if doc (append (help--lines doc) (list "")) nil)
     (help--bindings-lines buffer))))

(defun help--apropos-lines (pattern)
  "Every name matching PATTERN, with the first line of what it says."
  (let ((functions nil)
        (variables nil))
    (mapc (lambda (name)
            (when (string-match-p pattern name)
              (setq functions
                    (append functions
                            (list (concat "  " (help--pad name 28)
                                          (help--first-line
                                           (function-doc (help--symbol name)))))))))
          (all-functions))
    (mapc (lambda (name)
            (when (string-match-p pattern name)
              (setq variables
                    (append variables
                            (list (concat "  " (help--pad name 28)
                                          (help--first-line
                                           (variable-doc (help--symbol name)))))))))
          (all-variables))
    (if (and (null functions) (null variables))
        (list (concat "Nothing matches " pattern "."))
        (append
         (list (concat "Names matching " pattern) "")
         (if functions (append (list "Functions") functions (list "")) nil)
         (if variables (append (list "Variables") variables) nil)))))

(defun help--where-is-lines (name)
  "Where the command NAME can be reached from."
  (let ((keys (where-is (help--symbol name))))
    (if (null keys)
        (list (concat name " has no binding.")
              ""
              (concat "Run it by name with `M-x " name "'."))
        (list (concat name " is on " (help--join-words keys))))))

(defun help--for-help-lines ()
  "The help keys, read off `help-key' so they cannot go stale."
  (list
   "Help"
   ""
   (concat "  " (help--pad (concat help-key " f") 10) "describe a function")
   (concat "  " (help--pad (concat help-key " v") 10) "describe a variable")
   (concat "  " (help--pad (concat help-key " k") 10) "describe a key -- press it and be told")
   (concat "  " (help--pad (concat help-key " w") 10) "where a command is bound")
   (concat "  " (help--pad (concat help-key " b") 10) "every binding in effect")
   (concat "  " (help--pad (concat help-key " M") 10) "this buffer's mode")
   (concat "  " (help--pad (concat help-key " a") 10) "search every name")
   (concat "  " (help--pad (concat help-key " m") 10) "a manual page")
   (concat "  " (help--pad (concat help-key " e") 10) "the message log")
   ""
   "Half-way through a prefix, the help key says what it continues with:"
   (concat "  C-x " (help--last-key help-key)
           "   lists everything `C-x' leads to.")
   ""
   "In this buffer: RET or K follows the name under the cursor, l goes back,"
   "q closes it."))

;; ---------------------------------------------------------------------------
;; Showing one
;; ---------------------------------------------------------------------------

(defun help--build (page)
  "The lines of PAGE, which is (KIND . ARGUMENTS)."
  (let ((kind (car page))
        (rest (cdr page)))
    (cond ((string= kind "function") (help--function-lines (nth 0 rest)))
          ((string= kind "variable") (help--variable-lines (nth 0 rest)))
          ((string= kind "key") (help--key-lines (nth 0 rest) (nth 1 rest) (nth 2 rest)))
          ((string= kind "bindings") (help--bindings-lines (nth 0 rest)))
          ((string= kind "prefix") (help--prefix-lines (nth 0 rest)))
          ((string= kind "mode") (help--mode-lines (nth 0 rest)))
          ((string= kind "apropos") (help--apropos-lines (nth 0 rest)))
          ((string= kind "where-is") (help--where-is-lines (nth 0 rest)))
          ((string= kind "help") (help--for-help-lines))
          (t (list (concat "No such help page: " (help--name kind)))))))

(defun help--draw (page)
  "Put PAGE in the help buffer, without touching the history.

Built first and shown after, which is the order that needs no thinking about:
whatever a page reads, it reads while the editor is still as the reader left
it. The pages that describe a buffer do not rely on that -- they carry the
buffer's name, because going *back* to one happens from inside the help buffer
-- but a page added later might, and this way it is right by default."
  (let ((lines (help--build page)))
    (buffer-create help-buffer-name 'help-mode)
    (switch-to-buffer help-buffer-name)
    (set-buffer-read-only nil)
    (clear-buffer)
    (insert (help--join lines))
    (set-buffer-read-only t)
    (goto-char 0)
    (setq help--current page)))

(defun help--goto (page)
  "Show PAGE, remembering the one being left."
  (when help--current
    (setq help--history (cons help--current help--history))
    ;; Trimmed here rather than when walking back: a stack that only ever grew
    ;; would hold every page of a long session, and nothing would ever say so.
    (when (> (length help--history) help-history-length)
      (setq help--history (help--take help--history help-history-length))))
  (help--draw page))

(defun help--take (items count)
  "The first COUNT of ITEMS."
  (let ((out nil)
        (left count))
    (mapc (lambda (item)
            (when (> left 0)
              (setq out (append out (list item)))
              (setq left (- left 1))))
          items)
    out))

(defcommand help-back () nil
  "Go back to the page before this one. Bound to `l' in a help buffer."
  (if (null help--history)
      (message "No earlier help page")
      (let ((previous (car help--history)))
        (setq help--history (cdr help--history))
        ;; `help--draw' rather than `help--goto': going back must not push the
        ;; page being left, or `l' would walk between the last two pages for
        ;; ever and never reach the third.
        (help--draw previous))))

;; ---------------------------------------------------------------------------
;; The commands
;; ---------------------------------------------------------------------------

(defun help-function (name)
  "Show what the function NAME is and does."
  (help--goto (list "function" (help--name name))))

(defun help-variable (name)
  "Show what the variable NAME is for."
  (help--goto (list "variable" (help--name name))))

(defun help-apropos (pattern)
  "Show every name matching PATTERN, a regular expression."
  (help--goto (list "apropos" pattern)))

(defun help-where-is (name)
  "Show where the command NAME is bound."
  (help--goto (list "where-is" (help--name name))))

(defun help--function-candidates (input)
  "Function names matching INPUT, best first."
  (fuzzy-filter input (all-functions)))

(defun help--variable-candidates (input)
  "Variable names matching INPUT, best first."
  (fuzzy-filter input (all-variables)))

(defun help--command-candidates (input)
  "Command names matching INPUT, best first."
  (fuzzy-filter input (all-commands)))

(defcommand describe-function () nil
  "Ask which function to describe, completing over every one there is."
  (minibuffer-read "Describe function:"
                   (lambda (name) (unless (string= name "") (help-function name)))
                   'help--function-candidates
                   nil))

(defcommand describe-variable () nil
  "Ask which variable to describe, completing over every one there is."
  (minibuffer-read "Describe variable:"
                   (lambda (name) (unless (string= name "") (help-variable name)))
                   'help--variable-candidates
                   nil))

(defcommand where-is-command () nil
  "Ask which command to locate, and say which keys reach it."
  (minibuffer-read "Where is command:"
                   (lambda (name) (unless (string= name "") (help-where-is name)))
                   'help--command-candidates
                   nil))

(defcommand apropos () nil
  "Ask for a pattern and list every name matching it."
  (minibuffer-read "Apropos (regexp):"
                   (lambda (pattern) (unless (string= pattern "") (help-apropos pattern)))
                   nil
                   nil))

(defcommand describe-key () nil
  "Press a key sequence and be told what it runs.

The sequence is read by the editor's own resolution rather than by this
module, so what is reported is what would have happened -- the mode's binding
where a mode has one, the transient map's where a question is being asked, and
`C-u' named as the prefix argument it is. Nothing is run, and a key that would
have typed a character does not."
  (message "Describe key: press a key sequence")
  (read-key-sequence 'help--describe-key-answer))

(defun help--describe-key-answer (keys target source)
  "Show the page for KEYS, which the editor says runs TARGET from SOURCE."
  (help--goto (list "key" keys (help--name target) source)))

(defun describe-prefix-keys (prefix)
  "Show what PREFIX continues with.

Called by the editor when the help key is pressed half-way through a sequence,
which is the only way this is reached: there is no key of its own to press."
  (help--goto (list "prefix" prefix)))

(defcommand describe-bindings () nil
  "Show every binding in effect in this buffer."
  (help--goto (list "bindings" (current-buffer))))

(defcommand describe-mode () nil
  "Show this buffer's mode and the keys it binds."
  (help--goto (list "mode" (current-buffer))))

(defcommand help-for-help () nil
  "Show what the help keys are."
  (help--goto (list "help")))

;; ---------------------------------------------------------------------------
;; Following a name
;; ---------------------------------------------------------------------------

(defun help--symbol-at-point ()
  "The name under the cursor, or nil."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (if (null bounds)
        nil
        (buffer-substring (nth 0 bounds) (nth 1 bounds)))))

(defcommand help-follow () nil
  "Describe the name under the cursor.

A function if it is one, a variable otherwise -- the order a reader expects,
since a page naming `insert' means the command and a page naming
`case-fold-search' means the setting."
  (let ((name (help--symbol-at-point)))
    (cond ((null name) (message "Nothing under the cursor"))
          ((functionp (intern name)) (help-function name))
          ((or (boundp (intern name)) (variable-doc (intern name))) (help-variable name))
          (t (message "Nothing known about %s" name)))))

;; ---------------------------------------------------------------------------
;; Keys
;; ---------------------------------------------------------------------------

(defun install-help-keys ()
  "Bind the help tree under `help-key'.

Run when this module loads, and again after changing `help-key'. Rebinding
rather than moving: the old keys keep whatever they were given, which is what
somebody who moved the prefix to get their Backspace back is asking for."
  (define-key nil (concat help-key " f") 'describe-function)
  (define-key nil (concat help-key " v") 'describe-variable)
  (define-key nil (concat help-key " k") 'describe-key)
  (define-key nil (concat help-key " w") 'where-is-command)
  (define-key nil (concat help-key " b") 'describe-bindings)
  (define-key nil (concat help-key " M") 'describe-mode)
  (define-key nil (concat help-key " a") 'apropos)
  (define-key nil (concat help-key " e") 'switch-to-messages)
  (define-key nil (concat help-key " ?") 'help-for-help)
  (define-key nil (concat help-key " " help-key) 'help-for-help)
  t)

(install-help-keys)

;; `C-h m' stays with the manual: a manual page is the other half of what
;; somebody presses a help key to find, and it was bound there first. This
;; module takes `M' for the mode, which is the one collision worth living with.

(define-key 'help-mode "q" '(close-buffer help-buffer-name))
(define-key 'help-mode "l" 'help-back)
(define-key 'help-mode "n" 'next-line)
(define-key 'help-mode "p" 'previous-line)
(define-key 'help-mode "K" 'help-follow)
(define-key 'help-mode "<ret>" 'help-follow)

;; The shape of a page, recovered from the structure: a heading is a word
;; alone at the left margin, and everything a page quotes is `like this'.
(add-syntax-rule 'help-mode "^[A-Za-z][A-Za-z0-9 -]*$" 'keyword)
(add-syntax-rule 'help-mode "`([^']+)'" 'type 1)
(add-syntax-rule 'help-mode "^  ([^ ]+)" 'function 1)

(log "End of the help.lisp")
