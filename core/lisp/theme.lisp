;;; theme --- choosing how faces are drawn.
;;;
;;; C-c t lists the themes installed, and Return applies one; `select-theme'
;;; does the same from init.lisp. User manual: the `themes' page.
;;;
;;; The list calls `select-theme': a theme is applied one way, and the list
;;; only chooses its name. A theme file is data, read when the theme is
;;; applied, so listing the themes reads only the directory. Every face is
;;; reset before a theme is applied, so that a sparse theme after a full one
;;; leaves nothing of it; the price is that themes do not compose.

(make-mode 'theme-list-mode)

(defconst theme-list-buffer-name "*Themes*"
  "The buffer the list of themes is shown in.")

(defconst theme-suffix "-theme"
  "What follows a theme's name in the symbol its file puts the faces on:
monochrome.lisp puts them on monochrome-theme.")

(defvar current-theme nil
  "The theme in force, by name, or nil when there is none. `select-theme' and
`reset-theme' set it.")

;; ---------------------------------------------------------------------------
;; What is installed
;; ---------------------------------------------------------------------------

(defun theme--directory ()
  "Where theme files were installed, or nil."
  (data-directory "themes"))

(defun theme--name-of (file)
  "The theme name a file is called, or nil if it is not a theme file."
  (let ((name (file-name-nondirectory file)))
    (if (string-match "^(.+)\\.lisp$" name)
        (match-string 1 name)
        nil)))

;; Read from the directory rather than from what has been loaded: a theme
;; file is data, and loading one to find out it exists would mean loading all
;; of them to list them.
(defun available-themes ()
  "The names of the themes installed, sorted."
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

;; nil both for a theme that is not there and for one whose file defines
;; nothing: once applied, the two cannot be told apart.
(defun theme--faces (name)
  "The face list of the theme called NAME, loading its file if need be, or nil."
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
  "Make the editor look like the theme called NAME, a symbol or a string, and
return t; nil, with a message, when there is no such theme. Every face is put
back with `reset-faces' first, so that nothing of the theme before is left."
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
  "Take the theme away: put the faces back with `reset-faces'."
  (reset-faces)
  (setq current-theme nil)
  (message "Default appearance"))

;; ---------------------------------------------------------------------------
;; Choosing one
;; ---------------------------------------------------------------------------

(defconst theme-list-name-column 2
  "The column a theme's name begins at in the list of themes, after the mark
and a space.")

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

;; Read off the screen, the way dired and the buffer list read theirs: a
;; table built beside the buffer can disagree with what is displayed, and
;; reading the screen cannot.
(defun theme-list--name-here ()
  "The theme named on the cursor's line, or nil."
  (let ((line (current-line)))
    (if (<= (length line) theme-list-name-column)
        nil
        (let ((name (substring line theme-list-name-column)))
          (if (member name (available-themes)) name nil)))))

(defcommand theme-list () nil
  "Show the themes installed, the one in force marked with a *, for Return to
apply one."
  (buffer-create theme-list-buffer-name 'theme-list-mode)
  (switch-to-buffer theme-list-buffer-name)
  (theme-list--draw)
  (goto-line 1))

(defcommand theme-list-select () nil
  "Apply the theme on the cursor's line."
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
  "Read the themes installed again, keeping the cursor where it is."
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
