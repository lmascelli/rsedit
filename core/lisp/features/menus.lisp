;;; menus --- the menu bar and the context menu the editor comes with.
;;;
;;; File, Buffers and Help on the menu bar, Undo, Cut, Copy and Paste in the
;;; context menu of every buffer, F10 for the bar and S-F10 for the context
;;; menu from the keyboard, and the faces a terminal draws them in.
;;; User manual: the `menus' page.
;;; Design: `doc/architettura-editor.typ, <mod-menus>`.

;; ---------------------------------------------------------------------------
;; File
;; ---------------------------------------------------------------------------

(menu-bar-add '("File" "Open File...") 'find-file)
(menu-bar-add '("File" "Open Directory...") 'dired)
(menu-bar-add '("File" "Open Recent...") 'find-recent-file)
(menu-bar-add '("File" "--save") nil)
(menu-bar-add '("File" "Save") 'save-buffer)
(menu-bar-add '("File" "Save As...") 'write-file)
(menu-bar-add '("File" "Save All") 'save-some-buffers)
(menu-bar-add '("File" "Revert") 'revert-buffer 'buffer-file-name)
(menu-bar-add '("File" "--close") nil)
(menu-bar-add '("File" "Close Buffer") 'kill-buffer)
(menu-bar-add '("File" "--quit") nil)
(menu-bar-add '("File" "Quit") 'quit)

;; ---------------------------------------------------------------------------
;; Buffers
;; ---------------------------------------------------------------------------

(defun menus--buffer-entries ()
  "The entries of the Buffers menu: one for each buffer but the prompt's,
the current one marked."
  (let ((here (current-buffer))
        (entries nil))
    (dolist (name (all-buffer-names))
      (unless (string= name "*Minibuffer*")
        (setq entries
              (cons (list (if (string= name here) (concat name "  <") name)
                          (list 'switch-to-buffer name))
                    entries))))
    (reverse entries)))

(menu-bar-add-dynamic "Buffers" 'menus--buffer-entries)
(menu-bar-add '("Buffers" "--list") nil)
(menu-bar-add '("Buffers" "List All Buffers") 'buffer-list)

;; ---------------------------------------------------------------------------
;; Help
;; ---------------------------------------------------------------------------

(menu-bar-add '("Help" "Manual...") 'manpage-prompt)
(menu-bar-add '("Help" "Guide") '(manpage "guide"))
(menu-bar-add '("Help" "Menus and the Mouse") '(manpage "menus"))
(menu-bar-add '("Help" "--describe") nil)
(menu-bar-add '("Help" "Describe" "Function...") 'describe-function)
(menu-bar-add '("Help" "Describe" "Variable...") 'describe-variable)
(menu-bar-add '("Help" "Describe" "Key...") 'describe-key)
(menu-bar-add '("Help" "Describe" "Bindings") 'describe-bindings)
(menu-bar-add '("Help" "Describe" "Mode") 'describe-mode)
(menu-bar-add '("Help" "--log") nil)
(menu-bar-add '("Help" "Messages") 'switch-to-messages)

;; ---------------------------------------------------------------------------
;; The context menu of every buffer
;; ---------------------------------------------------------------------------

(context-menu-add nil "Undo" 'undo)
(context-menu-add nil "Redo" 'redo)
(context-menu-add nil "--kill" nil)
(context-menu-add nil "Cut" 'kill-region 'use-region-p)
(context-menu-add nil "Copy" 'kill-ring-save 'use-region-p)
(context-menu-add nil "Paste" 'yank)
(context-menu-add nil "--select" nil)
(context-menu-add nil "Select All" 'mark-whole-buffer)

;; ---------------------------------------------------------------------------
;; Keys and faces
;; ---------------------------------------------------------------------------

(define-key nil "<f10>" 'menu-bar-open)
(define-key nil "S-<f10>" 'context-menu-open)

;; How a terminal draws the bar and the menus; a window draws its own.
(define-face 'menu nil nil '("reverse"))
(define-face 'menu-selected nil nil '("bold"))
(define-face 'menu-disabled "bright-black" nil '("reverse"))

(log "menus loaded")
(provide 'menus)
