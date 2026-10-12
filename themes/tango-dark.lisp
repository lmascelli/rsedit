;;; tango-dark --- The GNOME Tango palette, as a dark theme.
;;;
;;; Not an editor theme to begin with: Tango was the 2005 icon palette for the
;;; free desktop, nine hues in three tints each, drawn up so that artwork from
;;; different hands would look like one set. Terminals adopted the twelve
;;; brightest as their sixteen-colour defaults, which is why this looks
;;; familiar on a machine you have never configured.
;;;
;;; The oldest palette here, and the only one in the bundle that was designed
;;; for something other than reading code.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'tango-dark-theme 'faces
     '(;; The page itself
       (default             "#d3d7cf" "#2e3436" nil)
       (region              nil       "#204a87" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#888a85" nil       nil)
       (line-number-current "#eeeeec" nil       ("bold"))
       (mode-line           "#eeeeec" "#555753" ("bold"))
       (mode-line-inactive  "#888a85" "#555753" nil)
       (window-separator    "#555753" nil       nil)

       ;; The grammar
       (keyword             "#729fcf" nil       ("bold"))
       (type                "#8ae234" nil       nil)
       (function            "#fce94f" nil       nil)
       (builtin             "#ad7fa8" nil       nil)
       (string              "#e9b96e" nil       nil)
       (comment             "#888a85" nil       ("italic"))
       (doc-comment         "#babdb6" nil       ("italic"))
       (attribute           "#ad7fa8" nil       nil)
       (preprocessor        "#fcaf3e" nil       nil)
       (error               "#ef2929" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#eeeeec" "#c4a000" ("bold"))
       (results-here        nil       "#204a87" nil)
       (results-target      nil       "#555753" nil)
       (preview             "#888a85" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#eeeeec" nil       ("bold"))
       (manpage-underline   "#ad7fa8" nil       ("underline"))))
