;;; nord --- Arctic, north-bluish: sixteen tones, four groups.
;;;
;;; Four polar-night greys for the page, three snow-storm whites for the text,
;;; four frost blues for everything structural, and five aurora colours for
;;; everything that needs to stand out. The grouping is the design: a face
;;; belongs to one of the four, and which one says how loud it is.
;;;
;;; The result is the least shouty of the dark schemes here -- the accents are
;;; all low-saturation, so a screen of code reads as blue-grey with occasional
;;; colour rather than as a colour wheel.
;;;
;;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES) -- the same four
;;; arguments `set-face' takes, so a theme is written the way a face is set.

(put 'nord-theme 'faces
     '(;; The page itself
       (default             "#d8dee9" "#2e3440" nil)
       (region              nil       "#434c5e" nil)

       ;; Furniture: the gutter, the status line, the rule between windows
       (line-number         "#4c566a" nil       nil)
       (line-number-current "#eceff4" nil       ("bold"))
       (mode-line           "#e5e9f0" "#3b4252" ("bold"))
       (mode-line-inactive  "#4c566a" "#3b4252" nil)
       (window-separator    "#4c566a" nil       nil)

       ;; The grammar
       (keyword             "#81a1c1" nil       nil)
       (type                "#8fbcbb" nil       nil)
       (function            "#88c0d0" nil       nil)
       (builtin             "#81a1c1" nil       nil)
       (string              "#a3be8c" nil       nil)
       (comment             "#616e88" nil       ("italic"))
       (doc-comment         "#7b88a1" nil       ("italic"))
       (attribute           "#b48ead" nil       nil)
       (preprocessor        "#5e81ac" nil       nil)
       (error               "#bf616a" nil       ("bold"))

       ;; Marks: search and replace, result listings, inline previews
       (replace-match       "#e5e9f0" "#ebcb8b" ("bold"))
       (results-here        nil       "#434c5e" nil)
       (results-target      nil       "#3b4252" nil)
       (preview             "#4c566a" nil       ("italic"))

       ;; `man' pages, which have no grammar -- only emphasis
       (manpage-bold        "#eceff4" nil       ("bold"))
       (manpage-underline   "#81a1c1" nil       ("underline"))))
