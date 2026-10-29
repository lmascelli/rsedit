;;; help --- what the editor can tell you about itself.
;;;
;;; C-h f a function, C-h v a variable, C-h k a key, C-h b the bindings, C-h a
;;; the names that match, C-h ? the help keys. User manual: the
;;; `getting-help' page.
;;; Design: `doc/architettura-editor.typ, <mod-help>`.

(require 'commands)
(require 'debug)
;; Not `manpage': the manual's key and links are used when it is loaded.

(make-mode 'help-mode)

(defconst help-buffer-name "*Help*"
  "The buffer help pages are shown in, one for them all.")

(defvar help-key "C-h"
  "The key the help keys follow, written as `define-key' writes keys. Its last
key, after a prefix, lists where the prefix goes. After changing it,
`install-help-keys' binds the help keys under it.")

(defvar help--history nil
  "The pages visited before this one, most recent first, each as what builds
it again: (KIND ARGUMENTS...).")

(defvar help--current nil
  "The page being shown, in the same form as an entry of `help--history'.")

(defvar help-history-length 50
  "How many pages back `help-back' can go.")

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
  "The last key of SEQUENCE."
  ;; The key that asks a prefix where it goes, as the editor reads `help-key'.
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
;; Each returns a list of lines and touches no buffer.
;; ---------------------------------------------------------------------------

(defun help--signature (name)
  "How NAME is called, as a line, or nil."
  ;; A primitive has no parameter list to read; the first line of its
  ;; documentation is its call.
  (let ((arglist (function-arglist (help--symbol name))))
    (if arglist
        (concat "(" name " " (help--join-words arglist) ")")
        (let ((first (help--first-line (function-doc (help--symbol name)))))
          (if (string= (substring first 0 1) "(")
              (car (split-string first ":"))
              nil)))))

(defun help--argument (argument)
  "ARGUMENT, a (KIND PROMPT) of `command-args', in words."
  (let ((kind (symbol-name (car argument)))
        (prompt (help--trim (nth 1 argument))))
    (cond ((string= kind "count") "the prefix count")
          ((string= kind "raw-count") "the raw prefix argument")
          ((string= kind "region") "the region")
          (t (concat (cond ((string= kind "string") "a string")
                           ((string= kind "number") "a number")
                           ((string= kind "buffer") "a buffer name")
                           ((string= kind "file") "a file name")
                           (t kind))
                     " (" prompt ")")))))

(defun help--trim (text)
  "TEXT without the spaces at its end."
  (if (string-match " +$" text)
      (substring text 0 (match-beginning 0))
      text))

(defun help--join-list (items)
  "ITEMS joined with commas."
  (let ((out ""))
    (mapc (lambda (item)
            (setq out (if (string= out "") item (concat out ", " item))))
          items)
    out))

(defun help--join-words (words)
  "WORDS joined with single spaces."
  (let ((out ""))
    (mapc (lambda (word)
            (setq out (if (string= out "") word (concat out " " word))))
          words)
    out))

(defun help--manual-lines (name)
  "The lines naming the manual page of NAME, or nil."
  (let ((page (and (functionp 'manpage-home) (manpage-home name))))
    (if page (list "" (concat "Manual: " page)) nil)))

(defun help--macro-lines (name found)
  "The page for the macro NAME, FOUND being what `help--macro-doc' says of it."
  (append
   (list name
         ""
         (if (car found)
             (concat "(" name " " (help--join-words (car found)) ")")
             (concat "(" name ")"))
         ""
         "A macro: its arguments are not evaluated before it runs."
         "")
   (help--lines (or (nth 1 found) "Undocumented macro."))
   (help--manual-lines name)))

(defun help--function-lines (name)
  "The page for the function NAME, or for the macro of that name."
  (let* ((symbol (help--symbol name))
         (found (help--macro-doc symbol))
         (doc (if found nil (function-doc symbol))))
    (cond (found (help--macro-lines name found))
          ((null doc)
           (list (concat name " is not a function.")
                 ""
                 (concat "Nothing is bound to that name. `" help-key
                         " a' searches every name there is.")))
          (t
           (append
            (list name "")
            (let ((signature (help--signature name)))
              (if signature (list signature "") nil))
            (let ((keys (where-is symbol)))
              (if keys
                  (list (concat "Bound to: " (help--join-words keys)) "")
                  nil))
            (if (commandp symbol)
                (let ((arguments (command-args symbol)))
                  (list (if arguments
                            (concat "A command. Its arguments: "
                                    (help--join-list
                                     (mapcar 'help--argument arguments))
                                    ".")
                            "A command. It takes no arguments.")
                        ""))
                nil)
            (help--lines doc)
            (help--manual-lines name))))))

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
               ;; Documented but unbound is a state of its own, not a nil.
               (if (boundp symbol)
                   (format "Value: %s" (symbol-value symbol))
                   "Not set. Whatever reads it will fail until something does.")
               "")
         (help--lines doc)
         (help--manual-lines name)))))

(defun help--key-lines (keys target source)
  "The page for KEYS, which run TARGET from the keymap SOURCE."
  ;; Given, not looked up again: they come from the editor's own lookup.
  (append
   (list (concat "Key: " keys) "")
   ;; `null' first: a key bound to nothing has no source.
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
  "Every binding in effect in BUFFER, grouped by the keymap it comes from."
  ;; Named, not assumed: going back to the page, the help buffer is current.
  (let ((transient nil)
        (local nil)
        (mode nil)
        (global nil))
    (mapc (lambda (binding)
            (let ((line (concat "  " (help--pad (nth 0 binding) 14)
                                (help--name (nth 1 binding))))
                  (source (nth 2 binding)))
              (cond ((string= source "transient") (setq transient (append transient (list line))))
                    ((string= source "local") (setq local (append local (list line))))
                    ((string= source "mode") (setq mode (append mode (list line))))
                    (t (setq global (append global (list line)))))))
          (with-current-buffer buffer 'keymap-bindings))
    (let ((mode-name (with-current-buffer buffer (lambda () (help--name (major-mode)))))
          (local-name (with-current-buffer buffer
                        (lambda () (help--name (buffer-get 'local-keymap))))))
      (append
       (list (concat "Bindings in effect in " buffer " (" mode-name ")") "")
       (if transient
           (append (list "Transient -- until the question is answered") transient (list ""))
           nil)
       (if local
           (append (list (concat "Local -- " local-name ", this buffer only")) local (list ""))
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
  "Put PAGE in the help buffer, leaving the history alone."
  ;; Built before the help buffer is shown, from the editor as it was left.
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
    ;; Trimmed here, so that the stack does not grow for a whole session.
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
        ;; `help--draw', not `help--goto': going back does not push the page
        ;; left, or `l' would only swap the last two.
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
  "Ask for a regular expression and list every name that matches it."
  (minibuffer-read "Apropos (regexp):"
                   (lambda (pattern) (unless (string= pattern "") (help-apropos pattern)))
                   nil
                   nil))

(defcommand describe-key () nil
  "Press a key sequence and be told what it runs, and from which keymap.
Nothing is run."
  ;; Through `read-key-sequence': the editor's own lookup.
  (message "Describe key: press a key sequence")
  (read-key-sequence 'help--describe-key-answer))

(defun help--describe-key-answer (keys target source)
  "Show the page for KEYS, which the editor says runs TARGET from SOURCE."
  (help--goto (list "key" keys (help--name target) source)))

(defun describe-prefix-keys (prefix)
  "Show what PREFIX continues with. Run by the editor when the help key follows
a prefix."
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
  "Describe the name under the cursor, as a function if it is one and as a
variable if not; on the Manual: line, open that page."
  (let ((name (help--symbol-at-point)))
    (cond ((null name) (message "Nothing under the cursor"))
          ((and (functionp 'manpage) (string-prefix-p "Manual: " (current-line)))
           (manpage name))
          ((functionp (intern name)) (help-function name))
          ((or (boundp (intern name)) (variable-doc (intern name))) (help-variable name))
          (t (message "Nothing known about %s" name)))))

;; ---------------------------------------------------------------------------
;; Keys
;; ---------------------------------------------------------------------------

(defvar help--installed-key nil
  "The key the help keys were last bound under, or nil before they were.")

(defun help--key-bindings (key)
  "The help keys under KEY, as (KEY-AFTER-IT . COMMAND)."
  ;; `m' is the manual's, and goes with the rest when the manual is there.
  (append (list (cons "f" 'describe-function)
                (cons "v" 'describe-variable)
                (cons "k" 'describe-key)
                (cons "w" 'where-is-command)
                (cons "b" 'describe-bindings)
                (cons "M" 'describe-mode)
                (cons "a" 'apropos)
                (cons "e" 'switch-to-messages)
                (cons "?" 'help-for-help)
                (cons key 'help-for-help))
          (if (functionp 'manpage-prompt) (list (cons "m" 'manpage-prompt)) nil)))

(defun install-help-keys ()
  "Bind the help keys under `help-key', the manual's among them, and take away
those still bound under the key they were under before."
  (if (and help--installed-key (not (string= help--installed-key help-key)))
      (mapc (lambda (binding)
              (let* ((keys (concat help--installed-key " " (car binding)))
                     (found (key-binding keys)))
                ;; Only while it is still ours: a key you rebound stays.
                (if (and found (string= (help--name (nth 1 found))
                                        (symbol-name (cdr binding))))
                    (define-key nil keys nil))))
            (help--key-bindings help--installed-key)))
  (mapc (lambda (binding)
          (define-key nil (concat help-key " " (car binding)) (cdr binding)))
        (help--key-bindings help-key))
  (setq help--installed-key help-key)
  t)

(install-help-keys)

;; `M' is the mode's page: `m' is the manual's.

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
(provide 'help)
