;;; solarized-dark --- Ethan Schoonover's 1:16 palette, dark.
;;;
;;; The one scheme designed rather than picked: sixteen tones chosen in CIELAB
;;; so that the eight accents hold the same perceived lightness, and the four
;;; background tones differ only in lightness. That is why no accent shouts
;;; over the others here, and why the light variant is this file with the two
;;; ends of the base ramp swapped.
;;;
;;; Published 2011 and still the reference point every later scheme is
;;; described against.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'solarized-dark-theme 'faces
     '(;; The page itself
       (default             "#839496" "#002b36" nil)
       (region              nil       "#073642" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#586e75" nil       nil)
       (line-number-current "#93a1a1" nil       ("bold"))
       (mode-line           "#eee8d5" "#073642" ("bold"))
       (mode-line-inactive  "#586e75" "#073642" nil)
       (window-separator    "#586e75" nil       nil)

       ;; The grammar
       (keyword             "#859900" nil       nil)
       (type                "#b58900" nil       nil)
       (function            "#268bd2" nil       nil)
       (builtin             "#2aa198" nil       nil)
       (string              "#2aa198" nil       nil)
       (comment             "#586e75" nil       ("italic"))
       (doc-comment         "#657b83" nil       ("italic"))
       (attribute           "#6c71c4" nil       nil)
       (preprocessor        "#cb4b16" nil       nil)
       (error               "#dc322f" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#eee8d5" "#b58900" ("bold"))
       (results-here        nil       "#073642" nil)
       (results-target      nil       "#073642" nil)
       (preview             "#586e75" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#93a1a1" nil       ("bold"))
       (manpage-underline   "#2aa198" nil       ("underline"))))
