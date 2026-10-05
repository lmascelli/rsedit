;;; high-contrast-gray --- stark, high-contrast grayscale theme.
;;;
;;; Maximizes legibility using the full grayscale dynamic range. 
;;; Features sharp text whites, deep UI contrast, and selective 
;;; font attributes for instant visual parsing.

(put 'high-contrast-gray-theme 'faces
     ;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES)
     '((keyword          "#ffffff" nil       ("bold"))
       (type             "#d8d8d8" nil       ("underline"))
       (function         "#ffffff" nil       ("bold"))
       (builtin          "#ffffff" nil       ("bold"))
       (comment          "#9e9e9e" nil       ("italic"))
       (doc-comment      "#b8b8b8" nil       ("italic"))
       (string           "#cccccc" nil       nil)
       (attribute        "#d8d8d8" nil       ("italic"))
       (error            "#ffffff" "#5a0000" ("bold" "underline"))

       ;; Editor UI furniture
       (region           "#ffffff" "#4f4f4f" ("bold"))
       (mode-line        "#ffffff" "#222222" ("bold"))
       (mode-line-inactive "#808080" "#111111" nil)
       (window-separator "#ffffff" nil       ("bold"))

       ;; Module & tool marks
       (manpage-bold     "#ffffff" nil       ("bold"))
       (manpage-underline "#ffffff" nil       ("underline"))
       (compilation-here "#ffffff" "#383838" ("bold"))
       (compilation-target "#ffffff" "#383838" ("bold"))))