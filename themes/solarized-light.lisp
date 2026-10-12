;;; solarized-light --- Ethan Schoonover's 1:16 palette, light.
;;;
;;; The same sixteen tones as `solarized-dark', with the base ramp reversed:
;;; the accents are *identical* hex values in both, which is the whole claim
;;; of the design. Switching between the two changes the page and not the
;;; code's colours.
;;;
;;; The paper tone is #fdf6e3 rather than white on purpose -- a warm ground
;;; at 97% lightness rather than 100%.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'solarized-light-theme 'faces
     '(;; The page itself
       (default             "#657b83" "#fdf6e3" nil)
       (region              nil       "#eee8d5" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#93a1a1" nil       nil)
       (line-number-current "#586e75" nil       ("bold"))
       (mode-line           "#586e75" "#eee8d5" ("bold"))
       (mode-line-inactive  "#93a1a1" "#eee8d5" nil)
       (window-separator    "#93a1a1" nil       nil)

       ;; The grammar
       (keyword             "#859900" nil       nil)
       (type                "#b58900" nil       nil)
       (function            "#268bd2" nil       nil)
       (builtin             "#2aa198" nil       nil)
       (string              "#2aa198" nil       nil)
       (comment             "#93a1a1" nil       ("italic"))
       (doc-comment         "#839496" nil       ("italic"))
       (attribute           "#6c71c4" nil       nil)
       (preprocessor        "#cb4b16" nil       nil)
       (error               "#dc322f" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#586e75" "#b58900" ("bold"))
       (results-here        nil       "#eee8d5" nil)
       (results-target      nil       "#eee8d5" nil)
       (preview             "#93a1a1" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#586e75" nil       ("bold"))
       (manpage-underline   "#2aa198" nil       ("underline"))))
