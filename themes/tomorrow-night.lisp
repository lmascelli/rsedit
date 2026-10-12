;;; tomorrow-night --- Chris Kempson's restrained dark theme.
;;;
;;; The scheme Base16 grew out of: eight greys and eight accents, specified as
;;; a *structure* rather than as a list, so the same design could be rebuilt
;;; for any editor. Most schemes with sixteen slots inherit that idea from
;;; here.
;;;
;;; Muted throughout -- no accent exceeds about 70% saturation -- which makes
;;; it the quietest of the three near-black themes in this bundle.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'tomorrow-night-theme 'faces
     '(;; The page itself
       (default             "#c5c8c6" "#1d1f21" nil)
       (region              nil       "#373b41" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#969896" nil       nil)
       (line-number-current "#e0e0e0" nil       ("bold"))
       (mode-line           "#c5c8c6" "#282a2e" ("bold"))
       (mode-line-inactive  "#969896" "#282a2e" nil)
       (window-separator    "#373b41" nil       nil)

       ;; The grammar
       (keyword             "#b294bb" nil       nil)
       (type                "#f0c674" nil       nil)
       (function            "#81a2be" nil       nil)
       (builtin             "#8abeb7" nil       nil)
       (string              "#b5bd68" nil       nil)
       (comment             "#969896" nil       ("italic"))
       (doc-comment         "#b4b7b4" nil       ("italic"))
       (attribute           "#de935f" nil       nil)
       (preprocessor        "#de935f" nil       nil)
       (error               "#cc6666" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#c5c8c6" "#f0c674" ("bold"))
       (results-here        nil       "#373b41" nil)
       (results-target      nil       "#282a2e" nil)
       (preview             "#969896" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#e0e0e0" nil       ("bold"))
       (manpage-underline   "#8abeb7" nil       ("underline"))))
