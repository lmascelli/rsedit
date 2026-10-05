;;; soft-contrast-gray --- refined mid-contrast grayscale theme.
;;;
;;; Enhances readability by spreading the brightness spectrum while 
;;; maintaining a clean, pure grayscale aesthetic without harsh saturated colors.

(put 'soft-contrast-gray-theme 'faces
     ;; Entry format: (FACE FOREGROUND BACKGROUND ATTRIBUTES)
     '((keyword          "#e0e0e0" nil       ("bold"))
       (type             "#cccccc" nil       nil)
       (function         "#ffffff" nil       ("bold"))
       (builtin          "#e0e0e0" nil       nil)
       (comment          "#888888" nil       ("italic"))
       (doc-comment      "#a0a0a0" nil       ("italic"))
       (string           "#bbbbbb" nil       nil)
       (attribute        "#cccccc" nil       nil)
       (error            "#ffffff" "#4a2828" ("bold" "underline"))

       ;; Editor UI furniture
       (region           nil       "#444444" nil)
       (mode-line        "#ffffff" "#333333" ("bold"))
       (mode-line-inactive "#888888" "#222222" nil)
       (window-separator "#555555" nil       nil)

       ;; Module & tool marks
       (manpage-bold     "#ffffff" nil       ("bold"))
       (manpage-underline "#cccccc" nil       ("underline"))
       (compilation-here "#ffffff" "#3a3a3a" nil)
       (compilation-target "#ffffff" "#3a3a3a" nil)))