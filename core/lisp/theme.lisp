;;; theme --- choosing how faces are drawn.
;;;
;;; `M-x theme-list' shows what is installed and Return applies one.
;;; `(select-theme 'monochrome)' does the same from a configuration file --
;;; the same function, so there is one way a theme is applied and the buffer
;;; is only a way of choosing an argument.
;;;
;;; # What a theme is
;;;
;;; A file under `data/themes' that `put's an alist on a symbol named after it:
;;; `monochrome.lisp' defines `monochrome-theme'. Each entry is
;;; (FACE FOREGROUND BACKGROUND ATTRIBUTES), which is exactly what `set-face'
;;; takes -- so a theme is written the way a face is set and there is no second
;;; vocabulary.
;;;
;;; Loading a theme file changes nothing. It defines data; applying it is a
;;; separate act. That is what makes it safe for the selector to load one in
;;; order to find out its name.
;;;
;;; # Why every face is reset first
;;;
;;; Applying a theme puts every face back to how the editor ships it before
;;; setting the ones the theme names. Without that a theme only changes what it
;;; mentions, so switching from a colourful theme to a sparse one leaves the
;;; colourful one's choices behind -- and a monochrome theme that is still half
;;; blue is the failure that makes a theme system feel broken.
;;;
;;; The cost is that themes do not compose: a small theme cannot tweak a big
;;; one. That is the right trade while selecting a theme means "make the editor
;;; look like this" rather than "adjust it a bit".

(make-mode 'theme-list-mode)

(defconst theme-list-buffer-name "*Themes*"
  "The buffer the list of themes is shown in.")

(defconst theme-suffix "-theme"
  "What a theme's symbol is named: the file's name, then this.

`monochrome.lisp' defines `monochrome-theme'. Spelling it out here means the
two places that need to agree -- the loader and the selector -- agree by
construction.")

;; Which theme is in force, as a name string, or nil for the shipped
;; appearance. Read by the selector to mark a line, and by nothing else.
(setq current-theme nil)

;; ---------------------------------------------------------------------------
;; What is installed
;; ---------------------------------------------------------------------------

(defun theme--directory ()
  "Where theme files were installed, or nil."
  (data-directory "themes"))

(defun theme--name-of (file)
  "The theme name a file is called, or nil if it is not a theme file."
  (let ((found (string-match "^(.+)\\.lisp$" (file-name-nondirectory file))))
    (if found (nth 1 found) nil)))

(defun available-themes ()
  "The names of every theme installed, sorted.

Read from the directory rather than from what has been loaded: a theme file is
data and loading one to find out it exists would mean loading all of them to
list them."
  (let ((directory (theme--directory)))
    (if (or (null directory) (not (file-directory-p directory)))
        nil
        (let ((names nil))
          (mapc (lambda (file)
                  (let ((name (theme--name-of file)))
                    (if name (setq names (cons name names)))))
                (list-dir directory))
          (reverse names)))))

(defun theme--symbol (name)
  "The symbol a theme called NAME defines."
  (intern (concat name theme-suffix)))

(defun theme--faces (name)
  "The face list of the theme called NAME, loading its file if need be.

Returns nil if there is no such theme, or if its file defines nothing -- which
is the same answer, because a theme that sets no faces and a theme that is not
there are indistinguishable once applied."
  (let ((faces (get (theme--symbol name) 'faces)))
    (if faces
        faces
        (let ((directory (theme--directory)))
          (if (null directory)
              nil
              (progn
                ;; `eval-file' looks in `lisp-path', which is not where themes
                ;; live, so the file is read and evaluated directly.
                (let ((text (read-file-to-string
                             (concat (file-name-as-directory directory)
                                     name ".lisp"))))
                  (if text (eval-string text)))
                (get (theme--symbol name) 'faces)))))))

;; ---------------------------------------------------------------------------
;; Applying one
;; ---------------------------------------------------------------------------

(defun select-theme (name)
  "Make the editor look like the theme called NAME. Returns t, or nil if there
is no such theme.

NAME may be a symbol or a string, so `(select-theme 'monochrome)' in a
configuration file and the selector's Return are the same call.

Every face is put back to how the editor ships it first -- see the module
header on why."
  (let* ((name (if (stringp name) name (symbol-name name)))
         (faces (theme--faces name)))
    (if (null faces)
        (progn (message "No theme called %s" name) nil)
        (progn
          (reset-faces)
          (mapc (lambda (entry)
                  (set-face (nth 0 entry) (nth 1 entry) (nth 2 entry) (nth 3 entry)))
                faces)
          (setq current-theme name)
          (message "Theme %s" name)
          t))))

(defcommand reset-theme () nil
  "Go back to the appearance the editor ships with."
  (reset-faces)
  (setq current-theme nil)
  (message "Default appearance"))

;; ---------------------------------------------------------------------------
;; Choosing one
;; ---------------------------------------------------------------------------

(defconst theme-list-name-column 2
  "Where a theme's name starts. Column 0 is the mark, column 1 a space.")

(defun theme-list--draw ()
  "Fill the listing with one theme per line, marking the one in force."
  (set-buffer-read-only nil)
  (clear-buffer)
  (let ((themes (available-themes)))
    (if (null themes)
        (insert "No themes installed")
        (mapc (lambda (name)
                (insert (if (and current-theme (string= name current-theme)) "* " "  ")
                        name "\n"))
              themes)))
  (set-buffer-read-only t))

(defun theme-list--name-here ()
  "The theme named on the cursor's line, or nil.

Read off the screen, the way `dired' and the buffer list read theirs: a table
built beside the buffer can disagree with what is displayed, and reading the
screen cannot."
  (let ((line (current-line)))
    (if (<= (length line) theme-list-name-column)
        nil
        (let ((name (substring line theme-list-name-column)))
          (if (member name (available-themes)) name nil)))))

(defcommand theme-list () nil
  "Show the themes installed, and apply one. Bound to C-c t.

Return applies the theme on the cursor's line; the one in force is marked with
a `*'. n and p move, q puts the listing away."
  (buffer-create theme-list-buffer-name 'theme-list-mode)
  (switch-to-buffer theme-list-buffer-name)
  (theme-list--draw)
  (goto-line 1))

(defcommand theme-list-select () nil
  "Apply the theme on the cursor's line. Bound to RET in theme-list-mode."
  (let ((name (theme-list--name-here)))
    (if (null name)
        (message "No theme on this line")
        (if (select-theme name)
            ;; Redrawn so the mark moves to the line just chosen. The listing
            ;; is the only thing that knows it has changed.
            (let ((here (line-number-at-point)))
              (theme-list--draw)
              (goto-line here))))))

(defcommand theme-list-refresh () nil
  "Re-read the directory of themes, keeping the cursor where it is."
  (let ((here (line-number-at-point)))
    (theme-list--draw)
    (goto-line here)))

(define-key nil "C-c t" 'theme-list)
(define-key 'theme-list-mode "<ret>" 'theme-list-select)
(define-key 'theme-list-mode "g" 'theme-list-refresh)
(define-key 'theme-list-mode "n" 'next-line)
(define-key 'theme-list-mode "p" 'previous-line)
(define-key 'theme-list-mode "q" '(close-buffer theme-list-buffer-name))

;; The mark on the theme in force.
(add-syntax-rule 'theme-list-mode "^\\* (.*)$" 'keyword 1)

(log "theme loaded")
