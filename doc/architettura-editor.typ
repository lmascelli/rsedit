#import "style.typ": *

#show: preamble.with(
  title: "L'editor di rsedit",
  subtitle: [Architettura della libreria `rsedit_core` e del front-end TUI --- manuale per lo sviluppatore],
)

#set text(lang: "it")

// Le figure disegnano il codice in linea piccolo, così una casella può
// nominare un tipo e restare leggibile.
#let canvas = canvas.with(code-size: 7.4pt)
// Nelle celle strette il testo giustificato apre spazi enormi tra le parole.
#show table: set par(justify: false)

// Un estratto di codice: l'etichetta dice da dove viene. Gli estratti sono
// semplificati (commenti tolti, generici abbreviati), mai inventati: ogni riga
// corrisponde a una riga del sorgente indicato.
#let estratto(origine, corpo) = block(breakable: false, above: 0.9em, below: 0.9em)[
  #text(size: 8pt, fill: dim)[da #raw(origine) --- estratto semplificato]
  #v(-0.45em)
  #code-block-size.update(7.8pt)
  #corpo
  #code-block-size.update(9pt)
]

// Un rimando a una scheda del rapporto sui problemi (rapporto-problemi.typ).
#let rapporto(id) = text(size: 0.85em, fill: warm)[(→ #id)]

#outline(depth: 2, indent: auto)

// ===========================================================================
= Introduzione
// ===========================================================================

Questo manuale descrive la libreria `rsedit_core` (`core/src/`) e il front-end
a terminale che la usa (`src/`). È il
compagno di #emph[L'interprete Lisp di rsedit] (`architettura-interprete.typ`),
che descrive il linguaggio: qui l'interprete è dato per noto, e si spiega come
l'editor lo ospita, come trasforma un tasto in un comando, come conserva il
testo, come lavora in background e come descrive lo schermo a chi lo disegna.

La libreria è un editor #emph[senza testa]. Non apre un terminale, non legge la
tastiera e non disegna: riceve eventi già tradotti nel proprio vocabolario
(`KeyEvent`, `MouseEvent`, testo incollato, dimensioni) e restituisce, quando le
viene chiesto, una fotografia immutabile di ciò che andrebbe mostrato
(`FrameSnapshot`). Tutto ciò che sta in mezzo --- buffer, finestre, modi,
comandi, lavori di sfondo --- è in `rsedit_core`, e quasi tutto il
comportamento visibile all'utente è scritto in Lisp nei moduli di `core/lisp/`.

#key[
  Due confini reggono l'architettura. L'interprete è un crate a sé, `risp`, e
  non può nominare alcun tipo dell'editor: questo permette di provarlo e
  riusarlo da solo. E `ui/` non nomina nulla di `editor/`, come verificano i
  test (`tests::layering_tests`): questo permette di scrivere un renderer contro
  una fotografia invece che contro gli interni della libreria.
]

== Come leggere questo manuale

Il @panoramica-editor e il @ciclo-vita danno il quadro: gli strati, i thread,
che cosa succede dall'avvio al ciclo degli eventi. Poi il manuale segue il
percorso di un tasto: lo stato che attraversa (@stato), la risoluzione e
l'esecuzione del comando (@tasto, @comandi, @minibuffer), il testo che il
comando modifica (@buffer, @testo), i modi che lo interpretano (@modi), il
lavoro che continua in background (@sfondo) e la fotografia che il front-end
disegna (@frame). Gli ultimi capitoli sono di riferimento.

Gli estratti di codice sono presi dal sorgente e semplificati: commenti tolti,
parametri generici abbreviati, rami ripetitivi riassunti. Ogni riga di un
estratto corrisponde a una riga del file indicato sopra di esso; il percorso è
relativo a `core/src/`.

Ogni affermazione è stata letta nel codice; i comportamenti che dipendono
dall'esecuzione (avvio, undo, comandi di shell, colorazione) sono stati
osservati con piccoli programmi che usano la libreria come farebbe un
front-end.

== Comportamento e difetti

Questo manuale descrive il comportamento #emph[attuale] del codice, anche dove
quel comportamento è un difetto. I difetti sono descritti, con la riproduzione
e una correzione proposta, nel #emph[Rapporto sui problemi del codice]
(`rapporto-problemi.typ`). Dove il manuale descrive un punto che il rapporto
considera un difetto, un rimando come #rapporto("BUF-1") indica la scheda
corrispondente; quando il difetto viene corretto si aggiorna la descrizione
qui e si toglie la scheda là.

== Convenzioni

- I nomi in `monospazio` sono identificatori: tipi e funzioni Rust
  (`run_command_form`), primitive e variabili Lisp (`call-interactively`,
  `mouse-mode`), file (`editor/keys.rs`, relativo a `core/src/` salvo
  indicazione diversa).
- Nei diagrammi le caselle grigie sono dati o stato, quelle bianche funzioni o
  passi, quelle azzurre appartengono a un altro thread o al front-end, quelle
  rosate sono errori. Nei diagrammi di sequenza le frecce continue sono
  chiamate e quelle tratteggiate risposte.
- «Primitiva» è una funzione Rust registrata nell'ambiente Lisp; «comando» è
  una funzione (Rust o Lisp) registrata anche nel registro dei comandi, e quindi
  raggiungibile con `M-x` e capace di farsi chiedere gli argomenti.

// ===========================================================================
= Panoramica <panoramica-editor>
// ===========================================================================

== Gli strati

`lib.rs` elenca i moduli in ordine di dipendenza: ognuno usa quelli sopra di sé
e, di quelli sotto, solo ciò che la sua voce dichiara. La figura li mostra dal
basso verso l'alto, cioè nella direzione in cui si costruisce.

#canvas(height: 8.4cm)[
  #node(8.1cm, 0.4cm, 15.8cm, 0.7cm, [`src/` --- i front-end, il terminale (crossterm) e la finestra (egui): traducono gli eventi, disegnano la fotografia], fill: accent.lighten(85%))
  #group(0.1cm, 1.2cm, 16.0cm, 2.2cm, [uno strato: si usano a vicenda])
  #node(4.0cm, 1.9cm, 7.6cm, 0.7cm, [`editor/` --- `EditorState`, la facciata], fill: paper)
  #node(12.15cm, 1.9cm, 7.7cm, 0.7cm, [`primitives/` --- ciò che Lisp può chiamare])
  #node(4.0cm, 2.85cm, 7.6cm, 0.7cm, [`modes/` --- modi e lavori che li servono])
  #node(12.15cm, 2.85cm, 7.7cm, 0.7cm, [`feature/` --- minibuffer e isearch])
  #node(8.1cm, 3.95cm, 15.8cm, 0.7cm, [`managers/` --- lo stato, in compartimenti, ognuno sotto un lock], fill: paper)
  #node(8.1cm, 4.95cm, 15.8cm, 0.7cm, [`ui/` --- la descrizione dello schermo: finestre, layout, facce, `FrameSnapshot`. Non disegna nulla])
  #node(4.0cm, 5.95cm, 7.6cm, 0.7cm, [`input.rs`, `commands.rs` --- tasti, keymap, comandi])
  #node(12.15cm, 5.95cm, 7.7cm, 0.7cm, [`text/` --- ricerca, rettangoli, kill ring])
  #node(4.0cm, 6.95cm, 7.6cm, 0.7cm, [`paths.rs` --- le directory dell'editor])
  #node(12.15cm, 6.95cm, 7.7cm, 0.7cm, [`buffer/` --- testo, undo, cache], fill: paper)
  #node(8.1cm, 7.9cm, 15.8cm, 0.6cm, [`work/` --- il lavoro in background: il WorkManager, l'esecutore, i thread dedicati], fill: paper)
]

Fino a `managers/` le frecce sono omesse perché la regola è una sola: ogni
livello può usare tutti quelli disegnati sotto di lui. `work/` non conosce
l'editor: un buffer emette un suo argomento senza sapere chi lo ascolta
(@argomenti). I quattro moduli in alto sono uno strato solo, non un ordine:
si usano a vicenda --- `editor/` installa le primitive e avvia i lavori dei
modi, che agiscono sull'editor --- e la mappa li elenca per ciò che sono.
L'interprete non è nella figura: è il crate `risp`, che non conosce l'editor;
le sue espressioni compaiono da `input.rs` in su.

La regola ha quattro eccezioni, dichiarate. `buffer/` conserva alcuni tipi che
vengono da più in alto, le facce di `ui/` (negli overlay e nel testo virtuale)
e gli stati della colorazione e della scansione di `modes/` (nelle due cache
del testo, @cache-testo). `input.rs` lega i tasti a espressioni Lisp,
`ELispExp`, il cui tipo nomina `EditorState` di `editor/` (@keymap). `ui/`
compone le finestre sulla tabella dei buffer, `Buffers` di `managers/`, e
usa `ELispExp` per dare a Lisp l'id di una finestra e per leggere lo stato che
un modo mette nella riga di stato. E
`managers/` conserva i modi maggiori e le tabelle sintattiche di `modes/`, e le
espressioni Lisp che i comandi, i legami e gli hook registrano
(@compartimenti); le finestre, le facce e il tema che conserva vengono da
`ui/`, che gli sta sotto. Per ogni modulo da `work/` a `managers/`,
`layering_tests` tiene l'elenco di ciò che usa da più in alto: vuoto, tranne
per questi quattro. Un uso nuovo fa fallire il test finché non lo si aggiunge
lì e nella mappa di `lib.rs`.

== Dove sta cosa

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Percorso], [Contenuto]),
  [`lib.rs`], [Ordine dei moduli e superficie pubblica (`pub use`). Tutto il resto è `pub(crate)`.],
  [`paths.rs`], [Le directory dell'editor, configurazione e cache, secondo le convenzioni di ogni sistema: `app_dir` (@directory).],
  [`buffer/`], [`Buffer`, `BufferTrait`, `GapBuffer`, undo, mark, overlay, testo virtuale, cache di colorazione (`syntax.rs`) e di scansione (`scan.rs`), timbro del file su disco (`disk.rs`).],
  [`text/`], [Ciò che si può calcolare sul testo conoscendo solo `BufferTrait`: la ricerca (`search.rs`), le sessioni di ricerca incrementale e di sostituzione (`isearch.rs`, `replace.rs`), il kill ring, i rettangoli, le liste di posizioni (`results.rs`).],
  [`input.rs`], [`KeyEvent`, `KeyCode`, `MouseEvent`, `Keymap`, `TransientKeymap`, la lettura e la scrittura dei nomi dei tasti (`parse_key_sequence`, `describe_keys`), i legami scritti in Rust (`fill_default_keymaps`).],
  [`commands.rs`], [`ArgSpec`, `PrefixArg`, `Invocation`, `PendingCommand`, `CommandRegistry`: il vocabolario dei comandi.],
  [`managers/`], [I compartimenti dello stato: `Buffers`, `Windows`, `Modes`, `Commands`, `KillYank`, `Macros`, `History`, `Log`, `Runtime`.],
  [`editor/`], [`EditorState` e le porte dei compartimenti (`mod.rs`); la metà della facciata di ogni compartimento, nel file con il suo nome (`buffers.rs`, `windows.rs`, `modes.rs`, `commands.rs`, `kill_yank.rs`, `macros.rs`, `log.rs`, `runtime.rs`); ciò che coordina più cose, nei file che portano il nome di ciò che coordinano (`keys.rs`, `mouse.rs`, `frame.rs`, `boot.rs`, `settings.rs`, `replace.rs`, `results.rs`, `background.rs`, `work.rs`). Vedi @coppie.],
  [`primitives/`], [Le 318 primitive dell'editor, in 36 moduli, e `install_primitives`.],
  [`modes/`], [`MajorMode`, grammatica di colorazione (`syntax.rs`), tabella sintattica e scanner di espressioni (`sexp.rs`), i lavori di sfondo `Highlighter`, `Prescanner`, `FileWatcher`, `AutoSaver`.],
  [`feature/`], [Le due funzionalità che non possono dipendere da un file Lisp: il minibuffer e la ricerca incrementale.],
  [`work/`], [Il WorkManager: il nucleo dello scheduler, l'esecutore, il registro, i thread dedicati, il watchdog, gli argomenti e le opzioni (@sfondo).],
  [`ui/`], [Albero delle finestre, finestre flottanti, layout delle righe, facce e temi, `FrameSnapshot`.],
  [`tests/`], [Circa ottanta file di test dell'editor, più `perf/`.],
  [`../lisp/`], [I moduli Lisp distribuiti con l'editor (venti file).],
  [`../build.rs`], [Copia i moduli Lisp in `target/<profilo>/data/lisp`, accanto all'eseguibile.],
  [`../../src/`], [I front-end: `main.rs` (avvio), `tui.rs` (il terminale: ciclo degli eventi e disegno) e `gui.rs` (una finestra grafica, con egui).],
)

== La superficie pubblica

Un front-end vede soltanto ciò che `lib.rs` riesporta: `EditorState`,
`create_global_env`, `BufferTrait`, `GapBuffer`, i tipi di input (`KeyEvent`,
`KeyCode`, `KeyModifiers`, `MouseEvent`, `MouseKind`, `MouseButton`), i tipi
della fotografia (`FrameSnapshot`, `RenderableWindowView`, `Highlight`,
`GutterCell`, `Separator`, `Rect`, `Face`, `Style`, `Color`, `Theme`,
`NAMED_COLORS`), dall'interprete `Env`, `LispContext`, `Parser` ed `eval`, i
compartimenti `Buffers` e `Windows`, le directory dell'editor (`app_dir`,
`AppDir`, `DirOptions`, `DirError` e i nomi delle variabili `CONFIG_DIR`,
`CACHE_DIR`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, @directory), più
`mouse_mode`, `MOUSE_MODE`, `isolate_config_for_tests` e l'alias
`ELispExp<B>` (`LispExp<EditorState<B>>`). I metodi di `EditorState` che un
front-end usa sono pochi:

#ref-table(
  columns: (5.2cm, 1fr),
  header: ([Metodo], [Uso]),
  [`create_global_env::<B>()`], [Costruisce editor e ambiente Lisp, carica la configurazione.],
  [`handle_key_event(ev, &env)`], [Un tasto premuto.],
  [`handle_paste(text, &env)`], [Un incolla «bracketed», come un solo comando.],
  [`handle_mouse_event(ev, &env)`], [Un evento del mouse; risponde se ha cambiato qualcosa.],
  [`resize(env, w, h)`], [Nuove dimensioni del terminale.],
  [`snapshot(&env, w, h)`], [La fotografia da disegnare.],
  [`next_redraw_in(&env, &frame)`, `drag_scroll_in()`], [Quanto si può aspettare prima di ridisegnare anche senza eventi.],
  [`tick(&env)`], [Il tempo è scaduto: esegue ciò che era in attesa (callback di sfondo, scorrimento durante il trascinamento).],
  [`on_background_change(sveglia)`], [Una funzione che il lavoro di sfondo chiama quando ha cambiato lo schermo: sveglia il ciclo del front-end (@utenti-sfondo).],
  [`is_running()`], [Falso dopo `quit`.],
  [`shutdown(entro)`], [All'uscita, anche per un panic: ferma il lavoro di sfondo e scrive nel file le ultime righe del log.],
  [`enable_log_file`, `log_diagnostic`, `set_echo_message`, `take_backtrace_suffix`], [Diagnostica.],
)

== I thread

L'editor usa più thread, ma uno solo esegue i comandi. La figura mostra chi
esiste; tutti tranne il principale appartengono al WorkManager (@sfondo), e
portano il suo numero nel nome.

#canvas(height: 6.2cm)[
  #group(0.1cm, 0.3cm, 7.6cm, 5.6cm, [thread principale (comandi)])
  #node(3.9cm, 1.2cm, 6.8cm, 0.8cm, [ciclo degli eventi della TUI], fill: accent.lighten(85%))
  #node(3.9cm, 2.4cm, 6.8cm, 0.8cm, [`handle_key_event` → `run_command_form`])
  #node(3.9cm, 3.6cm, 6.8cm, 0.8cm, [hook, callback dovute (`tick`), `snapshot`])
  #node(3.9cm, 4.9cm, 6.8cm, 1.0cm, [l'unico thread che apre prompt\ e su cui girano i comandi], fill: paper)

  #group(8.5cm, 0.3cm, 7.6cm, 2.0cm, [esecutore (`w1 executor`)])
  #node(12.3cm, 0.95cm, 6.8cm, 0.6cm, [`highlighter`, `prescanner` (classe alta)])
  #node(12.3cm, 1.75cm, 6.8cm, 0.6cm, [`file-watcher`, `auto-saver`, lavori Lisp])

  #group(8.5cm, 2.7cm, 7.6cm, 1.3cm, [watchdog (`w1 watchdog`)])
  #node(12.3cm, 3.35cm, 6.8cm, 0.6cm, [passi bloccati, thread finiti, file di log])

  #group(8.5cm, 4.4cm, 7.6cm, 1.5cm, [un thread dedicato per lavoro che si blocca])
  #node(12.3cm, 5.1cm, 6.8cm, 0.6cm, [comandi di shell, ricerche nei file])
  #carrow((7.3cm, 2.4cm), (8.9cm, 1.4cm), colour: accent)
]

La freccia è la casella con cui il thread dei comandi consegna il lavoro
all'esecutore. Tutti condividono lo stesso `EditorState` (clonato: ogni campo è
un `Arc`) e, i lavori che lo usano, lo stesso ambiente Lisp. Il carburante, il
permesso di `yield`, il contrassegno del comando per l'undo, i dati di
`string-match` e il segno di `in_worker` sono invece per thread (variabili
`thread_local!`). I lavori di sfondo non chiamano mai Lisp sul proprio thread
quando l'effetto deve toccare l'interfaccia: lasciano una #emph[callback
dovuta], che il thread principale esegue tra un comando e l'altro
(@utenti-sfondo). Fuori dal WorkManager restano i thread di `(spawn ...)`, che
l'interprete avvia da sé, e il breve `stop shell` di un comando di shell
fermato #rapporto("SHL-5").

// ===========================================================================
= Il ciclo di vita del programma <ciclo-vita>
// ===========================================================================

== Dal `main` al ciclo degli eventi

`src/main.rs` legge la riga di comando --- i file da aprire, `--gui`,
`--font FILE` e `--help` --- e chiama `create_global_env::<GapBuffer>()`; poi
attiva il file di log `rsedit.log` nella directory della cache (@directory),
apre ogni file della riga di comando valutando `(find-file PERCORSO)` ed entra
nel ciclo di un front-end: `tui_main`, il terminale, se lo standard input è un
terminale e `--gui` non è chiesto, altrimenti `gui_main`, una finestra
grafica. Un errore di avvio dell'interprete viene stampato e il programma
esce; un errore di `find-file` finisce nell'area dei messaggi. All'uscita,
anche per un panic, `ShutDownOnExit` chiama `shutdown`, che ferma il lavoro di
sfondo e scrive nel file le ultime righe del log.

#canvas(height: 8.4cm)[
  #lane(1.5cm, 0cm, 8.3cm, [`main.rs`], fill: accent.lighten(85%))
  #lane(5.2cm, 0cm, 8.3cm, [`create_global_env`], w: 3.2cm)
  #lane(8.9cm, 0cm, 8.3cm, [`EditorState`])
  #lane(11.7cm, 0cm, 8.3cm, [WorkManager], fill: accent.lighten(85%))
  #lane(14.6cm, 0cm, 8.3cm, [Lisp])

  #msg(1.5cm, 5.2cm, 1.2cm, [chiamata])
  #msg(5.2cm, 8.9cm, 1.65cm, [`EditorState::new()`])
  #msg(8.9cm, 11.7cm, 2.1cm, [`start`; `Highlighter`, `Prescanner`], colour: accent)
  #msg(5.2cm, 14.6cm, 2.6cm, [`bootstrap_vm`: radice + 86 primitive])
  #msg(5.2cm, 14.6cm, 3.1cm, [`lisp-path`, variabili di impostazione])
  #msg(5.2cm, 14.6cm, 3.6cm, [`install_primitives`, minibuffer, isearch])
  #seq-note(5.4cm, 3.85cm)[`init.lisp` scritto se manca]
  #msg(5.2cm, 14.6cm, 4.75cm, [`eval_file(init.lisp)` → i moduli])
  #msg(5.2cm, 11.7cm, 5.25cm, [`FileWatcher`, `AutoSaver`], colour: accent)
  #msg(5.2cm, 1.5cm, 5.75cm, [`(state, env)`], dash: "dashed")
  #msg(1.5cm, 14.6cm, 6.3cm, [`(find-file F)` per ogni file])
  #self-msg(1.5cm, 6.85cm, [`tui_main` o `gui_main`])
]

=== I passi di `create_global_env` <avvio>

+ `EditorState::new()` riempie la keymap globale con le poche associazioni
  scritte in Rust (`fill_default_keymaps`), crea ogni compartimento con il suo
  `Arc<RwLock<…>>`, avvia il WorkManager (`start`) e gli consegna i due lavori
  che girano sempre, `Highlighter` e `Prescanner`, che si svegliano quando un
  buffer o una grammatica cambiano.
+ `bootstrap_vm` crea l'ambiente radice con le 86 primitive di base e
  verifica l'interprete.
+ `lisp-path` diventa `("<directory dell'eseguibile>/data/lisp")`.
+ Ogni impostazione che l'editor legge (`editor/settings.rs`) riceve il suo
  valore iniziale, così che `(setq ...)` in una configurazione modifichi un
  valore che esiste già: per esempio `mouse-mode` (`nil`),
  `echo-message-timeout` (5), `minibuffer-width` (60), `minibuffer-height` (3),
  `window-separator` (`│`), `auto-save` e `watch-files` (`t`).
  `document_settings` dà a ognuna la documentazione che `C-h v` mostra. Viene
  creato il modo `fundamental-mode`.
+ `install_primitives`, `install_minibuffer` e `install_isearch` registrano le
  primitive, i comandi, il modo `minibuffer-mode` e il modo `isearch-mode`.
+ `rsedit-path` riceve il percorso dell'eseguibile.
+ Se manca, `init.lisp` viene creato con `DEFAULT_INIT_LISP` (vedi sotto).
+ `eval_file` valuta `init.lisp`, che carica i moduli Lisp. Un errore di
  valutazione ferma l'avvio, e `create_global_env` lo restituisce.
+ Il WorkManager riceve `FileWatcher` e `AutoSaver`, che a ogni passo leggono
  le loro impostazioni (`watch-files`, `watch-file-interval`, `auto-save`,
  `auto-save-interval`).

=== Le directory dell'editor <directory>

L'editor tiene due directory sue: quella della configurazione, con
`init.lisp`, e quella della cache, con ciò che può rifare, come il file di
log: cancellarla non perde nulla che conti. Le trova `app_dir`, in `paths.rs`:
le si chiede una directory per ciò a cui serve (`AppDir::Config`,
`AppDir::Cache`), e risponde con il suo posto su questo sistema, creandola se
manca e se `DirOptions::create` lo chiede. La usano `create_global_env`, per
`init.lisp`, e `main.rs`, per il log. Le due directory si trovano con la
stessa regola:

#ref-table(
  columns: (1.15fr, 1.4fr),
  header: ([Fonte, in ordine di precedenza], [Directory]),
  [`RSEDIT_CONFIG_DIR`, `RSEDIT_CACHE_DIR`], [usata così com'è, su ogni sistema],
  [`XDG_CONFIG_HOME`, `XDG_CACHE_HOME` (solo Unix, e solo se assoluta)], [la variabile, con `rsedit` aggiunto in fondo],
  [il sistema], [su Windows `%APPDATA%\rsedit` e `%LOCALAPPDATA%\rsedit\cache`; altrove, macOS compreso, `$HOME/.config/rsedit` e `$HOME/.cache/rsedit`],
)

Una variabile impostata ma vuota conta come non impostata. Quando nessuna
fonte risponde, `app_dir` dà `DirError::Undetermined`; gli altri errori sono
una directory che manca senza che si chieda di crearla (`NotFound`), un file
al suo posto (`NotADirectory`) e una creazione fallita (`CannotCreate`). Che
la directory si possa scrivere non viene controllato. La scelta tra le fonti,
`choose_dir`, riceve i valori invece di leggerli dall'ambiente: così i test
ne provano la precedenza senza scrivere nell'ambiente del processo, che tutti
i test vedono. `isolate_config_for_tests` dà invece a ogni processo di test
due directory sue, attraverso le due variabili `RSEDIT_*`.

Quattro decisioni:

+ *Su macOS, `~/.config` e `~/.cache`.* È lì che guarda chi usa un editor da
  terminale su un Mac: gli editor da cui arriva, Emacs con XDG, Neovim e
  Helix, tengono lì configurazione e cache. Scartato: `~/Library`, la
  convenzione delle applicazioni impacchettate.
+ *Su Windows la cache è locale.* `%APPDATA%` segue la persona da una
  macchina all'altra e viene copiato a ogni accesso. Una cache è proprio ciò
  che si può rifare, e quindi ciò che non va copiato: sta sotto
  `%LOCALAPPDATA%`.
+ *Su Windows le variabili `XDG_*` non contano.* Le imposta solo chi si porta
  dietro un'abitudine di Unix; preferirle alla risposta del sistema metterebbe
  la configurazione dove nessun programma Windows la cerca.
+ *Ogni processo di test ha le sue directory.* Costruire un editor crea la
  directory della configurazione e valuta ciò che contiene: sotto la `HOME` di
  chi lancia i test scriverebbe lì la sua configurazione, e alcuni test
  spostano `HOME`, che vale per tutto il processo mentre i test girano in
  parallelo. `isolate_config_for_tests` punta configurazione e cache a due
  directory temporanee del processo, una volta sola e prima del primo editor,
  attraverso le due variabili `RSEDIT_*`: la porta di una persona, così i test
  passano da `app_dir` invece di aggirarla. È pubblica per i test del crate
  binario, che il `cfg(test)` di una libreria non raggiunge.

=== Il file `init.lisp` predefinito

Il file `init.lisp` predefinito carica, in quest'ordine: `commands`, `debug`,
`common-keymaps`, `indent`, `minibuffer`, poi i moduli facoltativi
`rust-mode`, `risp-mode`, `cc-mode`, `python-mode`, `toml-mode`, `bash-mode`,
`dired`, `completion`, `preview`, `clipboard`, `electric-pair`,
`buffer-list`, `shell`, `manpage`, `help`, `occur`, `compile`, `theme`,
`vim`, `completion-at-point`, e imposta `dired-sort` a `'type` e `mouse-mode`
a `t`. L'ordine conta: `commands`
definisce `defcommand`, con cui sono scritti gli altri, e `debug` definisce
`message` e `report-error`.

Il file viene scritto solo se manca: chi ha già un `init.lisp` deve
aggiungere a mano le righe `(eval-file ...)` dei moduli nuovi #rapporto("AVV-5").

=== `eval_file`

`eval_file(nome, env)` cerca prima `nome` come percorso. Se non esiste e il nome
non contiene `/`, `\` o `.`, cerca `nome.lisp` in ogni directory di
`lisp-path`, in ordine. Il contenuto viene avvolto in `(progn …)`, letto con
#emph[una sola] chiamata a `Parser::next` e valutato dentro uno scope di
carburante (`begin_command`), alla radice della catena di ambienti
(`top_level`). Un file mancante produce una riga di log e il valore `nil`.

#estratto("editor/boot.rs")[
```rust
let content = format!("(progn {})", /* il testo del file, o di nome.lisp in lisp-path */);
let ast = if let Ok(ast) = Parser::new(&content).next() { ast } else {
    return Ok(ELispExp::nil());           // errore di sintassi: nessun messaggio
};
let _command = self.begin_command();             // uno scope di carburante
let value = eval(&ast, top_level(&env), self)?;  // alla radice dell'ambiente
Ok(ELispExp::proper_list(vec![value]))
```
]

L'involucro `(progn …)` fa del file un'unica espressione, e questo ha tre
conseguenze. Una parentesi chiusa di troppo chiude la `progn` in anticipo e il
resto del file non viene letto; una parentesi mancante, o un commento
sull'ultima riga senza a capo finale (la parentesi aggiunta finisce dentro il
commento), lascia la `progn` aperta e il lettore fallisce. In entrambi i casi
`eval_file` restituisce `nil` senza messaggi #rapporto("AVV-2"). Il primo errore
di #emph[valutazione], invece, interrompe il file da quel punto e risale come
errore.

=== Le decisioni <decisioni-avvio>

+ *I due lessici partono con l'editor.* L'evidenziatore e la prescansione
  sono sottomessi in `EditorState::new`, prima che esista un modo: un lavoro
  che aspetta un cambiamento non costa nulla, e uno mai avviato costerebbe un
  file senza colori e senza una ragione detta. Sono sottomessi una volta sola
  #rapporto("BKG-2").
+ *I due osservatori partono dopo `init.lisp`.* Il controllo dei file e il
  salvataggio automatico leggono le loro impostazioni a ogni passo, e sono
  sottomessi dopo la configurazione: ciò che dice conta dalla prima occhiata.
+ *Il file predefinito carica moduli e imposta variabili, niente di più.* È
  valutato con `?`, e i moduli che carica non sono sempre presenti --- un
  binario di test non ha `data/lisp` accanto ---: senza di loro `defcommand`
  non è una macro, una forma che la usa diventa una chiamata qualunque, e
  l'avvio si ferma. Un comando sta in un modulo o in Rust. È una stringa
  grezza, dove una barra rovesciata è una barra rovesciata; ed è una costante
  con un nome, perché un test la valuti in un ambiente nudo.
+ *Un file si valuta alla radice dell'ambiente.* Un `defun`, un `defvar` o il
  `setq` di un nome nuovo legano nell'ambiente in cui sono valutati, e dentro
  una funzione se ne vanno con lei. `eval_file` valuta alla radice
  (`top_level`), da dovunque venga la chiamata: un modulo caricato da una
  funzione, o con `M-:`, definisce ciò che definirebbe `init.lisp`.

== Il ciclo degli eventi della TUI

`tui_main` (`src/tui.rs`) attiva la modalità raw e l'incolla «bracketed»,
rileva la profondità di colore (`COLORTERM`, `TERM`), comunica le dimensioni
iniziali in `frame-width` e `frame-height`, e poi gira finché `is_running()` è
vero.

#canvas(height: 9.0cm)[
  #node(4.2cm, 0.45cm, 7.6cm, 0.7cm, [cattura del mouse = `mouse_mode(&env)`?])
  #carrow((4.2cm, 0.8cm), (4.2cm, 1.25cm))
  #node(4.2cm, 1.65cm, 7.6cm, 0.8cm, [se `dirty`: `snapshot` (lock, niente I/O)\ poi `render_frame` (I/O, niente lock)])
  #carrow((4.2cm, 2.05cm), (4.2cm, 2.5cm))
  #node(4.2cm, 2.95cm, 7.6cm, 0.9cm, [attesa = min(`next_redraw_in`, `drag_scroll_in`)\ `None` → blocca su `read()`])
  #carrow((4.2cm, 3.4cm), (4.2cm, 3.95cm))
  #node(4.2cm, 4.35cm, 7.6cm, 0.8cm, [`poll(attesa)`: arriva un evento prima?])
  #carrow((8.0cm, 4.35cm), (10.1cm, 4.35cm), label: [no], dy: -7pt)
  #node(12.9cm, 4.35cm, 5.4cm, 0.8cm, [`tick(&env)`; `dirty = true`])
  #carrow((4.2cm, 4.75cm), (4.2cm, 5.3cm), label: [sì], dx: 10pt)
  #node(4.2cm, 5.7cm, 7.6cm, 0.8cm, [`read()`], fill: accent.lighten(85%))
  #carrow((2.2cm, 6.1cm), (1.4cm, 6.75cm))
  #carrow((3.6cm, 6.1cm), (3.6cm, 6.75cm))
  #carrow((5.0cm, 6.1cm), (6.0cm, 6.75cm))
  #carrow((6.4cm, 6.1cm), (8.6cm, 6.75cm))
  #node(1.4cm, 7.35cm, 2.6cm, 1.1cm, [tasto (solo\ `Press`)\ `handle_key_event`])
  #node(4.0cm, 7.35cm, 2.4cm, 1.1cm, [`Resize`\ `resize`])
  #node(6.5cm, 7.35cm, 2.4cm, 1.1cm, [`Paste`\ `handle_paste`])
  #node(9.4cm, 7.35cm, 3.2cm, 1.1cm, [mouse\ `handle_mouse_event`\ (sporca solo se agisce)])
  #note(0.3cm, 8.1cm)[ogni ramo, tranne un mouse ignorato, imposta `dirty = true` e torna in cima]
  #segment((15.6cm, 4.35cm), (15.9cm, 4.35cm))
  #segment((15.9cm, 4.35cm), (15.9cm, 0.45cm))
  #carrow((15.9cm, 0.45cm), (8.0cm, 0.45cm))
]

Il cuore del ciclo, senza la gestione della cattura del mouse:

#estratto("../../src/tui.rs")[
```rust
while state.is_running() {
    if dirty {
        let (cols, rows) = terminal::size()?;
        frame = state.snapshot(&env, cols as usize, rows as usize);  // lock, niente I/O
        render_frame(&frame, depth)?;                               // I/O, niente lock
        dirty = false;
    }
    let wake = [state.next_redraw_in(&env, &frame), state.drag_scroll_in()]
        .into_iter().flatten().min();
    if let Some(remaining) = wake && !poll(remaining)? {
        state.tick(&env);         // l'attesa è scaduta: callback dovute, scorrimento
        dirty = true;
        continue;
    }
    match read()? {
        Event::Key(k) => if let Some(ev) = translate_key(k)
                             && k.kind == KeyEventKind::Press {
            state.handle_key_event(ev, &env); dirty = true;
        },
        Event::Resize(w, h) => {
            state.resize(env.clone(), w as usize, h as usize); dirty = true;
        }
        Event::Paste(text)  => { state.handle_paste(text, &env); dirty = true; }
        Event::Mouse(m) => if let Some(ev) = translate_mouse(m)
                               && state.handle_mouse_event(ev, &env) { dirty = true; },
        _ => (),
    }
}
```
]

Tre dettagli contano. Il ridisegno avviene solo se qualcosa è cambiato
(`dirty`): un terminale che riporta il mouse manda decine di movimenti al
secondo, e ridisegnare per ognuno farebbe sfarfallare lo schermo. L'attesa è
guidata dall'editor: se nulla è in sospeso `next_redraw_in` risponde `None` e il
ciclo si blocca su `read()` senza consumare CPU. E `tick` viene chiamato solo
quando l'attesa scade, non a ogni giro.

Un tasto che `translate_key` non sa tradurre --- `Delete`, `Home`, `End`,
`PageUp`, `PageDown`, `Insert`, i tasti funzione --- viene soltanto registrato
nel log: `KeyCode` non ha varianti per loro #rapporto("CMD-2"). Shift con un
carattere diventa la maiuscola; `BackTab` diventa `Tab` con Shift.

All'uscita normale la TUI disattiva la cattura del mouse e l'incolla, pulisce lo
schermo e lascia la modalità raw. Lo stesso ripristino non avviene se il ciclo
esce per un errore di I/O o per un panic #rapporto("TUI-2"). La TUI disegna
sullo schermo principale del terminale, non su quello alternativo
#rapporto("TUI-8").

// ===========================================================================
= Lo stato: `EditorState` e i compartimenti <stato>
// ===========================================================================

== La facciata

`EditorState<B: BufferTrait>` è una struttura di `Arc`: clonarla costa poco e
ogni clone vede lo stesso stato. I suoi campi sono privati, e ognuno è un
#emph[compartimento] di `managers/` sotto un `RwLock`, un dato piccolo con un
lock o un atomico proprio, o il WorkManager. La tabella dà i tipi senza l'`Arc`
che li condivide.

#ref-table(
  columns: (3.7cm, 5.0cm, 1fr),
  header: ([Campo], [Tipo], [Contenuto]),
  [`buffers`], [`RwLock<Buffers<B>>`], [Tabella nome → `Arc<RwLock<Buffer>>`, buffer corrente, ordine di recente uso.],
  [`windows`], [`RwLock<Windows>`], [Albero delle finestre affiancate, finestre flottanti, focus, prossimo id, trascinamento, ultimo clic.],
  [`modes`], [`RwLock<Modes<B>>`], [Registro dei modi maggiori, keymap globale, completamenti globali, modi per estensione, keymap transitoria, tasti di ripetizione, hook globali.],
  [`commands`], [`RwLock<Commands<B>>`], [Registro dei comandi, pila dei comandi in attesa di argomenti, argomento prefisso, ultimo comando e sua forma.],
  [`kill_yank`], [`RwLock<KillYank>`], [Kill ring, ultimo yank, flag «il comando precedente ha ucciso/incollato», ultimo rettangolo ucciso, testo dovuto agli appunti di sistema.],
  [`macros`], [`RwLock<Macros>`], [Macro in registrazione, ultima macro, profondità delle riproduzioni, contatore.],
  [`history`], [`RwLock<History>`], [Storia dei prompt per chiave (100 voci per chiave predefinite).],
  [`log`], [`RwLock<Log>`], [Righe diagnostiche e file a cui sono copiate.],
  [`runtime`], [`RwLock<Runtime>`], [Colonna obiettivo, isearch e sostituzione in corso, `FuelMeter`, pila delle chiamate per i backtrace, tema in uso e tema di fabbrica, i lavori avviati da Lisp per nome.],
  [`echo_message`], [`RwLock<EchoMessage>`], [Testo dell'area messaggi con l'istante in cui è stato scritto.],
  [`pending_keys`], [`RwLock<Vec<KeyEvent>>`], [Tasti di una sequenza non ancora completa.],
  [`owed`], [`RwLock<Vec<`\ `OwedCallback>>`], [Callback chieste dai lavori di sfondo e non ancora eseguite (@callback-dovute).],
  [`current_results`], [`RwLock<Option<String>>`], [Il buffer di risultati che `next-error` percorre.],
  [`grammar_epoch`], [`AtomicU64`], [Quante volte è cambiata una grammatica: il timbro di una colorazione.],
  [`syntax_changed`], [`Topic`], [L'argomento `syntax-changed`, risolto una volta (@argomenti).],
  [`work`], [`WorkManager<`\ `EditorState<B>>`], [Il lavoro asincrono: i lavori dell'esecutore e i thread dedicati (@sfondo).],
  [`background_change`], [`BackgroundChange`], [Se un lavoro ha cambiato lo schermo dall'ultima fotografia, e come dirlo al front-end (@utenti-sfondo).],
  [`running`], [`AtomicBool`], [Falso dopo `quit`.],
  [`shell_commands`], [`AtomicUsize`], [Quanti comandi di shell stanno girando.],
  [`command_errors`], [`AtomicUsize`], [Quanti comandi sono falliti: la riproduzione di una macro si ferma al primo.],
)

`EditorState` implementa `Debug` e `PartialEq` solo perché `LispContext` li
richiede; nessuno dei due è pensato per essere usato #rapporto("DOC-2").

== Le regole dei compartimenti <regole-compartimenti>

Ogni compartimento le segue; la documentazione di `managers/mod.rs` le
riassume.

#ref-table(
  columns: (5.0cm, 1fr),
  header: ([Regola], [Perché]),
  [Un compartimento non raggiunge un altro compartimento.], [Riceve i valori che gli servono e restituisce risposte (`Windows::scroll_focused` riceve un numero di righe, non un buffer). Così solo la facciata tiene due lock insieme.],
  [Non consegna guardie.], [Si accede allo stato solo con una closure (`windows(|w| …)`), e il lock non sopravvive alla domanda.],
  [Non valuta mai.], [Nessun compartimento riceve un `Env`. Può conservare valori Lisp (il corpo di un hook), mai eseguirli: Lisp può rientrare nell'editor da qualunque primitiva.],
  [Risponde con un verdetto, non con un effetto.], [Quando un'operazione ha conseguenze altrove (un buffer da ripuntare, una finestra da rifocalizzare) restituisce un enum come `BufferRemoved` o `WindowRemoved`, e la facciata agisce.],
  [Non fa nulla di lento con il lock preso.], [Niente disco, rete o attese. Registrare una diagnostica aggiunge una riga alla lista e basta: il file lo scrive il watchdog con `Log::flush`, che tiene il lock della lista solo per copiare le righe.],
)

Le prime quattro regole servono alla correttezza: ognuna toglie un modo in
cui due pezzi dello stato si possono vedere in disaccordo. L'ultima serve alla
latenza, ed è quella che si rompe per sbaglio, perché romperla non costa nulla
finché il disco è veloce e l'albero piccolo. Il segnale è un lock tenuto
attraverso una chiamata di cui non si controlla la durata: una lettura o una
scrittura, un'allocazione grande quanto un file, una callback. Se non si sa
dire quanto dura il corpo, si copia fuori ciò che serve e si rilascia prima.

La regola che nasce da tutte queste, e che vale anche fuori dai compartimenti,
è una sola: #emph[mai tenere un lock attraverso una chiamata all'interprete].
`run_hook` copia la lista degli hook e rilascia `modes` prima di valutarli;
`command_specs` restituisce dati posseduti perché `call-interactively` valuta
Lisp subito dopo. Un `RwLock` non è rientrante: un hook che chiama `add-hook`
mentre lo stesso thread tiene il lock di lettura del registro si blocca per
sempre, ogni volta.

== I compartimenti <compartimenti>

Ogni compartimento è un tipo di `managers/`, sotto un `RwLock` della facciata,
e tiene un'invariante che i suoi campi presi uno per uno non terrebbero.

#ref-table(
  columns: (3.2cm, 1fr),
  header: ([Compartimento], [Che cosa garantisce]),
  [`Buffers`], [La tabella non è mai vuota, e `current` nomina sempre un buffer della tabella: `make_current` e `bind_current` rifiutano un nome assente, `rename` e `remove` spostano `current` nello stesso passo.],
  [`Windows`], [Il focus nomina sempre una finestra che esiste, e l'ultima finestra affiancata non si chiude.],
  [`Modes`], [Le metà globali e quelle dei modi --- keymap, sorgenti di completamento, hook --- si leggono sotto lo stesso lock, nell'ordine in cui la risoluzione le consulta.],
  [`Commands`], [L'argomento prefisso appartiene a un comando solo, e i comandi in attesa di argomenti si chiudono in ordine inverso a quello in cui sono cominciati.],
  [`KillYank`], [I flag «il comando precedente ha ucciso, ha incollato» valgono rispetto all'anello e si spostano insieme, una volta per comando.],
  [`Macros`], [I tasti di una registrazione diventano l'ultima macro in un passo solo, e nessun tasto riprodotto viene registrato, a qualunque profondità.],
  [`History`], [Un cammino nella storia vale solo nell'anello e nel prompt in cui è cominciato.],
  [`Log`], [Ogni riga arriva al file una volta e nell'ordine in cui è stata registrata, salvo le scritture che falliscono e le righe uscite dalla lista prima di essere scritte.],
  [`Runtime`], [Nessuna: raggruppa ciò che appartiene alla sessione e non a un documento (@decisioni-stato).],
)

La facciata raggiunge un compartimento con una #emph[porta] che esegue una
closure: `windows(|w| …)` per leggere, `windows_mut(|w| …)` per cambiare. Il
lock non sopravvive alla domanda, e dentro la closure non si richiama la
facciata: un `RwLock` non promette che una seconda lettura sullo stesso thread
riesca; si può chiamare ciò che non prende lock, come fa `set_mode`, che sposta
il contatore delle grammatiche dentro la porta di `modes`. `Log` è il solo che
prende il lock da sé: `Log::append` e `Log::flush` ricevono il `RwLock<Log>`,
perché il WorkManager registra senza tenere un editor, che lo contiene.

== Le coppie con `editor/` <coppie>

`managers/` ed `editor/` si accoppiano per nome, con una regola sola.

- `managers/X` è il compartimento: i campi sotto un lock e le operazioni che
  non hanno bisogno d'altro.
- `editor/X` è la sua metà della facciata: i metodi di `EditorState` che danno
  alle operazioni di X un nome da editor (`command_specs` per
  `Commands::specs`, perché la facciata ha anche nomi di buffer e di finestre) e
  ciò che usa X insieme a un altro compartimento, al testo di un buffer o
  all'interprete.
- Le porte dei lock stanno tutte in `editor/mod.rs`, accanto ai campi: è il
  solo file che prende i lock dei compartimenti. Una porta sola ne tiene due,
  `windows_mut_with_buffers`, per `snapshot`, che compone ogni finestra sul
  buffer che mostra (@ordine-lock).
- Un metodo che inoltra una chiamata sola al compartimento X sta in
  `editor/X.rs`, chiunque lo chiami: i cinque del tema in `runtime.rs`, quelli
  che il mouse chiede alle finestre in `windows.rs`.
- Un compartimento senza metodi di facciata non ha un file in `editor/`: oggi
  `history`, le cui porte bastano.
- Chi ha bisogno di un compartimento solo, primitive comprese, usa la porta:
  `ctx.modes(|m| …)` in una primitiva è previsto.

Gli altri file di `editor/` portano il nome di ciò che coordinano --- `keys.rs`,
`mouse.rs`, `frame.rs`, `boot.rs`, `settings.rs`, `replace.rs`, `results.rs`,
`background.rs` --- e tengono solo ciò che mette insieme più cose: un
compartimento e il testo di un buffer, l'interprete o un altro compartimento.
`work.rs` è la facciata del WorkManager, che non è un compartimento: lo si
raggiunge intero (`work()`), perché non ha un lock che si possa prendere fuori
ordine o tenere attraverso l'interprete. L'implementazione di `LispContext`,
il contratto dell'editor con l'interprete, sta in `mod.rs`.

== L'ordine dei lock <ordine-lock>

Quando un'operazione deve tenere più lock insieme, li prende in quest'ordine.
Sono due a farlo. `snapshot` tiene `windows`, in scrittura, `buffers` e i
buffer che compone, attraverso la sola porta che apre due compartimenti,
`windows_mut_with_buffers`. La risoluzione di un tasto tiene `pending_keys`
mentre legge il buffer corrente e poi interroga `modes`, e lo prende in
scrittura per togliere una mappa `Release`. Ogni altro percorso copia fuori ciò
che gli serve e rilascia prima di prendere il lock seguente; `echo_message` e
gli altri compartimenti si prendono da soli.

#canvas(height: 3.4cm)[
  #node(1.4cm, 1.0cm, 2.4cm, 0.8cm, [ambiente Lisp\ (`Env`)], fill: paper)
  #node(5.0cm, 1.0cm, 2.8cm, 0.8cm, [`pending_keys`])
  #node(8.6cm, 1.0cm, 2.4cm, 0.8cm, [`windows`])
  #node(11.8cm, 1.0cm, 2.2cm, 0.8cm, [`buffers`])
  #node(15.0cm, 1.0cm, 1.9cm, 0.8cm, [un `Buffer`])
  #carrow((2.6cm, 1.0cm), (3.6cm, 1.0cm))
  #carrow((6.4cm, 1.0cm), (7.4cm, 1.0cm))
  #carrow((9.8cm, 1.0cm), (10.7cm, 1.0cm))
  #carrow((12.9cm, 1.0cm), (14.05cm, 1.0cm))
  #node(15.0cm, 2.6cm, 1.9cm, 0.7cm, [`modes`])
  #carrow((15.0cm, 1.4cm), (15.0cm, 2.25cm))
  #note(0.2cm, 2.15cm)[le variabili si leggono #emph[prima] di ogni altro lock e mai con uno preso: `snapshot` legge le impostazioni,\ poi l'area messaggi, poi `windows` e `buffers` insieme; la risoluzione di un tasto tiene `pending_keys`,\ legge il modo del buffer corrente e solo dopo prende `modes`.]
]

`buffers` ha una particolarità: nel percorso comune non viene tenuto mentre si
blocca un buffer. `with_buffer` e `with_current_buffer` clonano l'`Arc` del
buffer dalla tabella, #emph[rilasciano] la tabella e solo allora bloccano il
buffer. Così chi scrive in un buffer non chiude fuori dalla tabella gli altri
thread --- l'evidenziatore e lo scanner la percorrono mentre l'utente scrive.
Soltanto `snapshot` tiene la tabella e i singoli buffer insieme, nell'ordine
indicato.

== Accesso ai buffer

#canvas(height: 3.6cm)[
  #node(2.4cm, 0.6cm, 4.4cm, 0.8cm, [`with_current_buffer_mut(f)`])
  #carrow((4.6cm, 0.6cm), (6.4cm, 0.6cm))
  #node(9.1cm, 0.6cm, 5.2cm, 0.8cm, [`buffers(|b| b.current_handle())`\ lock della tabella, clona l'`Arc`], fill: paper)
  #carrow((11.7cm, 0.6cm), (12.6cm, 0.6cm))
  #node(14.3cm, 0.6cm, 3.2cm, 0.8cm, [tabella rilasciata])
  #carrow((14.3cm, 1.0cm), (14.3cm, 1.75cm))
  #node(12.6cm, 2.2cm, 6.6cm, 0.8cm, [`handle.write()` → `f(&mut Buffer)` → rilascio])
  #note(0.2cm, 1.7cm)[`with_buffer(nome, f)` fa lo stesso per nome,\ e risponde `None` se il buffer non esiste;\ il buffer corrente esiste sempre.]
]

`Buffers` non è mai vuoto: nasce con `*scratch*` corrente. Le funzioni che
cambiano quale buffer è corrente passano tutte da `make_current` o, per
`set-buffer`, che agisce su un buffer senza mostrarlo, da `bind_current`, che
non tocca l'ordine dei buffer recenti: tutte e due rifiutano un nome assente
dalla tabella (@decisioni-stato).

== L'editor come contesto dell'interprete

`EditorState<B>` è il `T: LispContext` dell'interprete:

#ref-table(
  columns: (5.4cm, 1fr),
  header: ([Metodo del trait], [Implementazione]),
  [`consume_fuel(n)`], [`FuelMeter` di `Runtime` (clonato fuori dal lock); `Exhausted` diventa `OutOfFuel`.],
  [`log_diagnostic(msg)`], [`Log::append`: la riga va nella lista; il file lo scrive il watchdog (`Log::flush`).],
  [`with_unwind(cleanup)`], [Esegue la pulizia di un `unwind-protect` con il carburante che resta e, finito quello, con una riserva di 100.000 passi, che non passa alla valutazione ordinaria e non si ricarica fino al prossimo scope esterno.],
  [`begin_thread_evaluation()`], [`arm_thread()`: il nuovo thread riceve il budget configurato.],
  [`push_call_frame` / `pop_call_frame` / `call_frame_depth` / `truncate_call_frames`], [La pila `call_stack: Vec<String>` di `Runtime`, che `report_error` legge come backtrace.],
)

Lo scope di carburante dell'editor è `begin_command()`: lo aprono
`run_command_form`, ogni hook, `report_error`, ogni callback dovuta, ogni
funzione di `after-resize-hook` ed `eval_file`. Solo l'apertura più esterna
ricarica il budget (10.000.000 di passi), quindi un `eval-file` dentro un
comando consuma il budget del comando, e così `pre-command-hook`, che gira
dentro lo scope del comando. `post-self-insert-hook` e `post-command-hook`,
invece, girano dopo che lo scope si è chiuso: ogni loro funzione riparte con il
budget pieno.

== Le decisioni <decisioni-stato>

+ *Lo stato è diviso in compartimenti, ognuno sotto un lock.* Un compartimento
  raccoglie i campi che sono un fatto in più pezzi. Sotto lock separati si
  vedrebbero a metà di un cambiamento --- il focus che nomina una finestra che
  il layout ha già tolto --- e ogni operazione che ne tocca due dovrebbe
  prenderli in un ordine scritto altrove. Sotto un lock solo non c'è un ordine
  da rispettare, e il disaccordo non si può vedere.
+ *`current` non nomina mai un buffer assente.* Nella tabella, `current` è una
  chiave e `recency` una lista di chiavi: due pezzi su tre valgono solo
  rispetto al terzo. `current` si cambia solo con `make_current` e
  `bind_current`, che rifiutano un nome assente, e `rename` e `remove` lo
  spostano nello stesso passo, non lasciandolo al chiamante. Un nome assente
  diventato corrente farebbe fallire la ricerca successiva con un panic, e con
  l'editor andrebbe perso ciò che non era salvato nelle altre finestre.
+ *`Buffers` non legge il testo e non conosce le finestre.* Ciò che un buffer
  contiene è di `Buffer`, e chi lo mostra sono le finestre, che ne tengono il
  nome. Il rapporto è a senso unico, e i due compartimenti non hanno bisogno
  l'uno del lock dell'altro: quando un nome cambia o un buffer sparisce,
  `BufferRenamed` e `BufferRemoved` lo dicono e la facciata ripunta le finestre.
  `rename` non tocca neppure il campo `name` del buffer: bloccare un buffer
  tenendo la tabella metterebbe la tabella in coda a chi sta scrivendo nel
  buffer.
+ *I campi di `Commands` sono la vita di un comando.* Il registro dice che cosa
  può eseguire `M-x` e con quali argomenti, l'argomento prefisso è il conteggio
  scritto prima, la pila è ciò che resta da raccogliere, gli ultimi due campi
  sono ciò che resta al comando successivo. Si scrivono tutti nel corso di un
  tasto, e l'ordine in cui lo sono è il protocollo. Che il prefisso sia ancora
  in lettura è un campo a sé: «nessun argomento» e «un argomento finito» sono
  stati diversi, e una cifra significa cose diverse nei due.
+ *Gli hook stanno con i modi, non con i comandi.* Sembrano macchinario dei
  comandi, ma un hook appartiene a un modo: quelli registrati per tutti i modi
  sono la metà globale di `MajorMode::hooks`, come la keymap globale lo è di
  `MajorMode::keymaps`.
+ *Guardare indietro nella storia non fa perdere ciò che si scriveva.* Il primo
  passo indietro mette da parte il testo del prompt, e andare avanti oltre la
  voce più recente lo restituisce. Altrimenti, per tornare a ciò che si stava
  scrivendo, bisognerebbe annullare il prompt e ricominciare.
+ *La storia non ricorda tutto.* Non ricorda la stringa vuota, che è come si
  esce da un prompt, né la ripetizione della voce più recente: lo stesso
  comando eseguito quattro volte lascia una voce, o tornare indietro di un
  passo costerebbe quattro tasti. Una ripetizione più vecchia resta dov'è e si
  aggiunge di nuovo in testa: spostarla riordinerebbe la storia sotto l'utente.
  Il tetto, cento voci come in Emacs, è per prompt, così un prompt usato di
  continuo non spinge fuori le voci di un altro. Ai due capi di un anello il
  prompt resta com'è, invece di ricevere una copia di sé: il tasto deve poter
  dire «sei alla fine».
+ *Anello e flag di `KillYank` stanno insieme.* L'anello da solo sarebbe un
  campo. Ne fanno un compartimento le due domande che Emacs fa a ogni kill e a
  ogni yank: il comando precedente ha ucciso? ha incollato? Una serie di `C-k`
  deve fare una voce sola, e `yank-pop` deve sapere quale testo ha appena
  inserito uno yank. Sotto lo stesso lock «accodare o aggiungere» e «flag ed
  estensione dello yank» sono ciascuno un'operazione sola.
+ *Due flag per domanda.* Un comando alza `killed_this` mentre gira, e
  `roll_over` lo sposta in `killed_last` quando è finito; lo stesso per lo
  yank. Con un flag solo, un comando non potrebbe chiedere che cosa ha fatto il
  precedente senza vedere anche ciò che ha appena fatto lui. Le due coppie si
  spostano insieme, perché un comando chiede di tutte e due.
+ *Il rettangolo ucciso non sta nell'anello.* Un rettangolo è una forma, e
  l'anello contiene testo: con un rettangolo nell'anello ogni yank dovrebbe
  chiedersi che forma ha, e un blocco copiato da un terminale non si
  distinguerebbe dalle righe di cui è fatto. È un posto e non un anello suo,
  perché non c'è uno `yank-pop` dei rettangoli; e due kill di rettangolo di
  seguito sono due rettangoli, non un blocco che sullo schermo non c'è mai
  stato.
+ *La lista del log e il suo file sono una cosa sola.* Il file è lo specchio
  della lista: abilitarlo scrive tutto ciò che è stato registrato fino a quel
  momento, così il file contiene la storia intera e non solo ciò che è venuto
  dopo. Funziona solo se lista e file si sistemano insieme, sotto lo stesso
  lock: altrimenti una riga registrata in mezzo da un altro thread arriverebbe
  troppo tardi per la storia e troppo presto per il file.
+ *Il file del log non lo scrive chi registra.* Registrare aggiunge una riga e
  la conta come non scritta, e basta: niente disco, quindi registrare costa lo
  stesso con il disco pieno o con la directory di casa in rete, e nessun
  comando, nessun passo dello scheduler aspetta una scrittura. Scrive
  `Log::flush`, che il watchdog chiama a ogni giro, un quarto di secondo per
  impostazione: su un thread che non è bloccato quando il resto lo è, che è
  quando un log serve. Un'ultima volta alla chiusura.
+ *`Log` prende il suo lock da sé e non va mai in panic.* È il solo
  compartimento con funzioni che ricevono il proprio `RwLock`, perché il
  WorkManager registra senza tenere un editor (@sfondo). Una scrittura che
  fallisce si perde, lasciando un buco nel file, e un lock avvelenato si usa
  così com'è: `flush` gira sul watchdog, che riporta i panic degli altri e non
  deve averne di suoi --- con il disco pieno, salvare un file registra una
  diagnostica la cui scrittura fallisce a sua volta.
+ *Una macro registra tasti, non comandi.* Non ogni tasto diventa un comando:
  `C-u` e le cifre li prende il lettore dell'argomento prefisso, e
  `zap-to-char` legge il suo carattere con una cattura di tasti, senza un
  secondo comando. Un registratore che guardasse i comandi riprodurrebbe
  `zap-to-char` come l'attesa di un carattere che non arriva. I tasti seguono
  poi ciò che significano #emph[ora]: si cambia un legame e la macro fa la cosa
  nuova, come in Emacs. E si scrivono gratis: `describe_keys` e
  `parse_key_sequence` sono inversi, quindi una macro è una stringa nella
  sintassi di `define-key`.
+ *Registrazione e riproduzione stanno insieme.* Finire una registrazione
  sposta i tasti nel posto dell'ultima macro: separati, un lettore in mezzo
  vedrebbe due copie o nessuna. E se un tasto si registri è una domanda su
  registrazione e riproduzione insieme: una macro che ne chiama un'altra
  registra la chiamata, non ciò in cui si espande. I comandi del registratore
  (`NOT_RECORDED`) non si registrano, perché i tasti che chiudono una
  registrazione si premono mentre gira: l'elenco è per nome e sta accanto allo
  stato, perché quali comandi siano del registratore è una domanda sua.
+ *Le riproduzioni annidate hanno un tetto.* Una riproduzione è una chiamata
  Rust attraverso il gestore dei tasti, quindi una macro che chiama se stessa
  fa crescere la pila, che finisce molto prima del carburante --- se ne
  spendono pochi passi per livello --- e una pila finita chiude il processo
  invece di dare un errore. Sedici livelli bastano: l'annidamento legittimo,
  una macro che chiama una macro con nome che ne chiama un'altra, ne usa due o
  tre.
+ *Le metà globali stanno con il registro dei modi.* La keymap di un modo e
  quella globale si consultano in quest'ordine a ogni tasto, e le sorgenti di
  completamento del modo e quelle globali si fondono a ogni completamento.
  Separate, chi le legge dovrebbe sapere che i posti sono due e prenderli
  nell'ordine giusto. La keymap transitoria e i tasti di ripetizione stanno
  sotto lo stesso lock, così risolvere un tasto prende un lock solo.
+ *Gli hook e le sorgenti si copiano fuori prima di chiamarli.* Le liste di
  `Modes` contengono espressioni Lisp, e niente in `Modes` le chiama: ogni
  lettura le copia e rilascia il lock. Un hook che chiama `add-hook`,
  `define-key`, `make-mode` o `add-syntax-rule` vuole questo lock in scrittura,
  e con il lock tenuto si bloccherebbe ogni volta, su un thread solo, con un
  Lisp qualunque.
+ *Il cammino tra le keymap si scrive una volta.* `binding_for`, `is_prefix` e
  `bindings` rispondono sui legami in effetto accanto alla risoluzione che
  l'editor esegue, e ne condividono il cammino. Un secondo cammino scritto
  altrove sarebbe una seconda risposta a «che cosa fa questo tasto», e le due
  divergerebbero la prima volta che l'ordine cambiasse. Ogni legame porta con
  sé la mappa da cui viene (`BindingSource`): «`q` esegue `quit-window` qui,
  perché il buffer è in `dired-mode`» è quasi tutto ciò che vuole sapere chi
  cerca un tasto.
+ *I modi per nome di file sono una lista ordinata.* I modelli si
  sovrappongono --- `\.rs$` e `^Cargo\.` corrispondono entrambi a `Cargo.rs`
  --- e quale vince deve averlo deciso qualcuno, non un hash: vince il primo
  dichiarato.
+ *I tasti di ripetizione si dichiarano.* La regola che verrebbe in mente,
  «dopo una sequenza che finisce con K, un K da solo ripete», farebbe di `f`
  dopo `C-x C-f` un altro `find-file`.
+ *`Runtime` raggruppa per durata, non per un'invariante.* Gli altri
  compartimenti tengono insieme un fatto in più pezzi; questi campi no: nessuna
  operazione ne tocca due, e metterli insieme non chiude nessuna finestra di
  disaccordo. Il guadagno è più piccolo: la facciata non porta otto lock
  indipendenti, e chi cerca lo stato dell'editor, distinto da quello di un
  documento, ha un posto solo. Il tema sta qui in mancanza di meglio: è
  dell'interfaccia, e si sposterà quando ci sarà un compartimento suo.
+ *Il carburante non sta sotto il lock di `Runtime`.* `FuelMeter` si
  sincronizza da sé: il budget vivo è una variabile del thread, e l'atomico
  dietro si legge solo quando si apre uno scope. Si tiene come `Arc` e si dà
  per clone, così chi apre uno scope, che dura un comando intero, non tiene il
  lock attraverso l'interprete.
+ *Le finestre non leggono i buffer.* Una finestra nomina un buffer e non sa
  altro. Dove un'operazione ha bisogno di qualcosa del buffer --- lo
  scorrimento il numero di righe, `C-l` la riga del punto --- riceve il numero:
  i due compartimenti non si tengono mai insieme, e le operazioni si provano
  senza un editor. Per lo stesso motivo un clic risponde con una colonna dello
  schermo, che `editor/mouse.rs` traduce in una colonna del buffer: una riga
  con testo virtuale è più larga della sua riga di testo.
+ *I doppi clic li conta l'editor.* Un terminale manda una pressione per volta
  e nient'altro: «questa è la seconda nello stesso punto» lo sa solo chi tiene
  la prima, e deve essere l'editor, perché in un front-end sarebbe un calcolo
  da rifare in ogni front-end. Due clic sono una coppia nella stessa cella
  entro 400 ms, un po' meno del `double-click-time` di Emacs. La stessa cella e
  non una vicina, perché un clic subito accanto al precedente sposta il cursore
  di un carattere, e leggerlo come doppio sarebbe l'errore più fastidioso. Dopo
  una coppia il primo clic si dimentica: tre pressioni sono una coppia e una
  singola.
+ *Un trascinamento si ricorda, e scorre a tempo.* È l'unico gesto del mouse
  che non è un evento solo: dove è cominciato decide che cosa significano gli
  eventi dopo, ovunque sia andato il puntatore. Un puntatore fermo fuori dalla
  finestra non manda eventi, ma la selezione deve continuare: per questo lo
  scorrimento durante il trascinamento è un timer di 60 ms, non una risposta
  al movimento.
+ *Una striscia divide il frame e non prende il focus.* `open_bottom` apre una
  finestra alta esattamente quanto chiesto sotto tutte le altre, che cedono
  ciascuna una parte dello spazio: una striscia è del frame, non di una
  finestra tagliata a metà. Il focus resta dov'è, perché la striscia si mostra
  mentre si scrive altrove --- i completamenti sotto un prompt --- e il focus
  che si spostasse renderebbe corrente il suo buffer: il tasto successivo
  finirebbe nella lista dei suggerimenti.
+ *I float sono nel giro delle finestre, ma non nel loro conto.* `all_ids`
  mette i float dopo le finestre affiancate, dal più vecchio, perché un float
  da cui il focus è uscito sarebbe altrimenti irraggiungibile. `count` conta
  solo le affiancate, perché decide se una finestra si può chiudere e che cosa
  annulla `C-x 1`, e un prompt disegnato sopra non divide nulla.
+ *Scorrere sposta la vista, e il punto la segue.* `C-v` sposta la vista e il
  punto la segue solo se ne è uscito, come in Emacs: spostare il punto e
  lasciare che il disegno lo insegua lo porterebbe al bordo invece di
  tenergli il posto sullo schermo. Due righe restano in comune tra una
  schermata e l'altra. La rotella segue la stessa regola nella finestra con il
  focus, e nelle altre lascia il punto dov'è: il punto è del buffer, e
  un'altra finestra può mostrare lo stesso buffer. La rotella porta l'ultima
  riga fino in cima, `C-v` solo fino in fondo: la prima guarda la fine di un
  file, il secondo lo legge.
+ *La facciata dà ai compartimenti nomi da editor.* `is_command` inoltra a
  `Commands::is_command`, e sembra un doppione; ma per lo più rinomina:
  `Commands::specs` è chiaro perché il tipo dice di che cosa parla, mentre
  `EditorState`, che ha anche nomi di buffer e id di finestre, ha bisogno di
  `command_specs`. Togliere gli inoltri metterebbe in ogni chiamata quale
  compartimento tiene che cosa, e spostare un campo tra due compartimenti
  cambierebbe ogni chiamante invece di un inoltro.
+ *Un inoltro sta con il suo compartimento.* Chi cerca che cosa l'editor
  offre su un compartimento lo trova in un file, e chi scrive un inoltro nuovo
  sa dove metterlo, chiunque sia a usarlo. I file di coordinamento restano
  fatti di coordinamento.
+ *Una porta sola tiene due compartimenti.* `snapshot` compone ogni finestra
  contro la tabella dei buffer, e ne sposta lo scorrimento: ha bisogno di
  `windows` in scrittura e di `buffers` insieme. La porta che li apre
  entrambi li prende nell'ordine dei lock, scritto così in un posto solo
  (@ordine-lock).
+ *Il contatore delle grammatiche sta accanto al registro, non dentro.* Una
  colorazione porta il timbro della grammatica sotto cui è stata calcolata, e
  aggiungere una regola non cambia il testo: senza il contatore nessuno se ne
  accorgerebbe. Ogni fotografia chiede a ogni buffer se la sua colorazione è
  indietro, e un atomico fuori dal registro risponde senza prenderne il lock.
  Si sposta solo sotto il lock in scrittura del registro, dopo il cambio della
  grammatica, e lo si legge sotto quel lock insieme alla grammatica: le due
  cose non si vedono mai in disaccordo. Una tabella sintattica che cambia
  emette `syntax-changed` e non sposta il contatore: è dell'altro lessico.
+ *Il messaggio e il suo istante stanno sotto un lock solo.* Con due lock, chi
  legge potrebbe accoppiare un messaggio nuovo all'istante del vecchio e
  nasconderlo appena scritto. Ogni messaggio fa ripartire il suo tempo, e un
  messaggio scaduto resta nello stato: è la vista che lo dimentica.
+ *Le callback dovute non sono un compartimento.* Ogni compartimento non sa
  nulla del Lisp, e quella lista ne è fatta: metterla dietro un compartimento
  romperebbe la regola, o chiederebbe una seconda rappresentazione di una
  chiamata, inventata per non dire che cos'è. Sta accanto a `current_results`,
  come stato che la facciata tiene da sé.
+ *Finestra con il focus e buffer corrente cambiano insieme.* Lo fanno in due
  modi soli: `show_in_focused_window` cambia il buffer e lascia il focus,
  `set_focused_window_id` sposta il focus e rende corrente ciò che la finestra
  mostra, e ogni spostamento del focus finisce in `follow_focus`, anche quando
  è il compartimento a spostarlo. Se i due andassero alla deriva, l'editor
  disegnerebbe il cursore in una finestra e scriverebbe in un'altra.
+ *In un buffer in sola lettura si scrive sotto il suo lock.* Una trascrizione
  --- l'uscita di un comando di shell, i risultati di una ricerca --- è in sola
  lettura perché nessuno ci scriva. `append_to_buffer` abbassa e rialza il
  flag dentro un solo lock in scrittura: non c'è un momento in cui il buffer è
  visibile e scrivibile. `dired` fa lo stesso dal Lisp, dove non può tenere un
  lock e deve fidarsi che nulla giri in mezzo.
+ *Un file che non c'è si apre vuoto e non modificato.* `new_buffer` con un
  percorso legge il file e fallisce se non può; un file che non esiste non è
  un errore, è il modo comune di crearne uno. `new_file_buffer` fa un buffer
  vuoto che ricorda dove andrà, non modificato: lasciarlo non perde nulla, e
  `C-x C-s` crea il file anche se non si è scritto niente.
+ *Un buffer chiuso con modifiche non salvate lascia la sua copia.* La copia
  del salvataggio automatico si butta quando ciò che teneva è nel file: a un
  salvataggio, e alla chiusura di un buffer senza modifiche. `close_buffer` non
  chiede nulla, e non chiedono `close-buffer` e `kill-buffer-without-saving`
  che lo chiamano: la copia può essere tutto ciò che resta del lavoro.

// ===========================================================================
= Dal tasto al comando <tasto>
// ===========================================================================

== Il vocabolario dei tasti <tasti>

Un `KeyEvent` è un `KeyCode` (`Char(c)`, `Backspace`, `Enter`, `Esc`, `Tab`, le
quattro frecce, e `None`, che nessuno costruisce) più i modificatori `ctrl`,
`alt`, `shift`. Canc, Inizio, Fine, PagSu, PagGiù e i tasti funzione non hanno
un `KeyCode` #rapporto("CMD-2"). Un `MouseEvent` dice che cosa ha fatto il
mouse (`MouseKind`: un pulsante premuto, rilasciato o trascinato, la rotella),
in quale cella del frame e con quali modificatori. Sono i tipi che un
front-end costruisce e consegna a `handle_key_event` e a `handle_mouse_event`.

Nelle associazioni i tasti si scrivono come in Emacs: `C-x`, `M-x`, `C-M-f`,
`S-<tab>`, con i nomi `<ret>`, `<esc>`, `tab`, `<backtab>`, `<backspace>`,
`<space>`, `<left>`, `<right>`, `<up>`, `<down>`. Un tasto ha al più un
prefisso di modificatori, quindi `M-C-x` e `C-S-a` non si possono scrivere
#rapporto("CMD-4"). Una sequenza è una stringa di tasti separati da spazi:
`"C-x C-f"`. `parse_key_sequence` legge queste stringhe e `describe_keys` le
riscrive nello stesso modo; stanno in `input.rs`, con i tipi.

La keymap globale scritta in Rust (`fill_default_keymaps`) contiene
pochissimo: `M-x` (`command-execute-prompt`), le frecce, Invio e i caratteri
stampabili da `' '` a `'~'`, legati a `(self-insert "c")`. Tutto il resto ---
`C-x C-f`, `C-s`, `C-k`, … --- viene da `common-keymaps.lisp` e dagli altri
moduli.

Solo i caratteri ASCII stampabili sono legati a `self-insert`. Il ripiego per
gli altri caratteri esiste --- una keymap può dare un comando a ogni carattere
che non lega, con `(define-key MAPPA "<char>" COMANDO)` --- ma la keymap
globale non lo usa: un carattere come «è» arriva alla risoluzione senza un
comando e produce `è is undefined` #rapporto("CMD-1").

== Le keymap <keymap>

Una `Keymap` associa sequenze di tasti a espressioni Lisp: un nome di comando
(`forward-char`), una forma che porta già i suoi argomenti
(`(self-insert "x")`) o una lambda. Il tipo è `ELispExp`, che
`run_command_form` esegue così com'è; ed è l'unica cosa che `input.rs` prende
da più in alto, perché il suo tipo nomina `EditorState`.

#estratto("input.rs")[
```rust
pub struct Keymap<B: BufferTrait> {
    bindings: HashMap<Vec<KeyEvent>, ELispExp<B>>,  // la sequenza intera
    prefixes: HashSet<Vec<KeyEvent>>,               // ogni prefisso proprio
    character_handler: Option<ELispExp<B>>,         // il ripiego dei caratteri
}
pub fn insert(&mut self, keys: Vec<KeyEvent>, ast: ELispExp<B>) {
    for len in 1..keys.len() {
        self.prefixes.insert(keys[..len].to_vec());
    }
    self.bindings.insert(keys, ast);
}
```
]

Ogni tasto chiede due cose a una keymap: se la sequenza è un legame (`get`) e
se è l'inizio di uno (`is_prefix`). Tutte e due sono la ricerca in una tabella
hash: `insert` registra i prefissi propri della sequenza insieme al legame, e
`remove` ricostruisce l'insieme dai legami che restano. `binding` aggiunge il
ripiego: un carattere senza modificatori che non è un legame né l'inizio di
uno va al `character_handler`, se c'è; un simbolo diventa `(simbolo "c")`,
ogni altra funzione `(funcall 'f "c")`. `set_character_handler` lo installa,
ed è ciò che fa `(define-key MAPPA "<char>" ...)`; con `nil` lo toglie.

Le keymap stanno nel compartimento `Modes`: la globale, quella di ogni modo e
la #emph[transitoria], consultata prima delle altre finché resta installata.
Una `TransientKeymap` è una `Keymap`, il messaggio da mostrare mentre è attiva
e che cosa fare di un tasto che non lega (`OnUnbound`, nella tabella della
risoluzione). La installano le domande a un tasto (`y-or-n`,
`save-some-buffers`), la vista predefinita di `query-replace`, la striscia dei
completamenti e la ripetizione: `TransientKeymap::repeating` costruisce la
mappa `[o]`, che `install_repeat_keymap` rimette dopo ogni comando con un tasto
di ripetizione.

=== Le decisioni <decisioni-keymap>

+ *I prefissi si conservano, non si cercano.* Ogni tasto chiede se la sequenza
  è l'inizio di un legame. Rispondere scorrendo i legami vorrebbe dire
  scorrere tutta la mappa, centinaia di voci per i soli caratteri, sul
  percorso tra il tasto e il carattere che compare. L'insieme cambia solo
  quando si definisce un legame, all'avvio o quando l'utente cambia la sua
  configurazione; la domanda si fa a ogni tasto.
+ *Legame e prefissi si registrano insieme.* Un legame senza i suoi prefissi
  sarebbe irraggiungibile: `C-x` da solo risulterebbe non definito, e la
  sequenza verrebbe buttata prima di arrivare a `C-x C-f`. Per questo i campi
  sono privati e `insert` li scrive tutti e due: lo stato sbagliato non si
  può costruire, invece di essere solo sconsigliato. Togliere un legame è
  raro, e `remove` ricostruisce i prefissi invece di contarli.
+ *Domande e offerte sono lo stesso meccanismo.* «Premi `y` o `n`» e «premi
  ancora `o`» sembrano due funzioni diverse, ma sono una keymap consultata per
  prima che poi se ne va: cambia solo `OnUnbound`. Essere una keymap, invece di
  un ciclo che legge le risposte, fa delle risposte comandi normali: si legano
  come tutto il resto, l'aiuto li descrive, e ognuno è un comando a sé per
  l'undo.
+ *Una mappa `Release` non ha un'uscita.* Se ne va dentro la risoluzione del
  tasto, mentre la sequenza è ancora sotto lock, dove non si può chiamare
  l'interprete. Quindi non deve portare con sé uno stato da ripulire: una
  mappa con una sessione dietro deve essere una `Refuse`, che finisce con i
  suoi tasti, dove un comando può ripulire. La vista predefinita di
  `query-replace` non segue la regola #rapporto("RIC-8").

== `handle_key_event`

#canvas(height: 11.6cm)[
  #node(4.6cm, 0.45cm, 6.4cm, 0.7cm, [`KeyEvent` dal front-end], fill: accent.lighten(85%))
  #carrow((4.6cm, 0.8cm), (4.6cm, 1.2cm))
  #node(4.6cm, 1.6cm, 6.4cm, 0.8cm, [`*key-capture-function*` non nil?])
  #carrow((7.8cm, 1.6cm), (9.6cm, 1.6cm), label: [sì], dy: -7pt)
  #node(12.9cm, 1.6cm, 6.4cm, 0.8cm, [`capture_key_sequence`: stessa risoluzione,\ poi `(funcall WATCHER KEYS TARGET SOURCE)`])
  #carrow((4.6cm, 2.0cm), (4.6cm, 2.45cm))
  #node(4.6cm, 2.85cm, 6.4cm, 0.8cm, [`read_prefix_argument`: `C-u`, cifre, `-`?\ (se la keymap locale non lega `C-u`)])
  #carrow((7.8cm, 2.85cm), (9.6cm, 2.85cm), label: [preso], dy: -7pt)
  #node(12.9cm, 2.85cm, 6.4cm, 0.7cm, [registrato per le macro; fine])
  #carrow((4.6cm, 3.25cm), (4.6cm, 3.7cm))
  #node(4.6cm, 4.15cm, 6.4cm, 0.9cm, [`resolve_key_sequence`:\ transitoria → locale → modo → globale])
  #carrow((7.8cm, 4.15cm), (9.6cm, 4.15cm), label: [prefisso], dy: -7pt)
  #node(12.9cm, 4.15cm, 6.4cm, 0.7cm, [tasti conservati in `pending_keys`; fine])
  #carrow((4.6cm, 4.6cm), (4.6cm, 5.05cm))
  #node(4.6cm, 5.45cm, 6.4cm, 0.8cm, [nessuna associazione?])
  #carrow((7.8cm, 5.45cm), (9.6cm, 5.45cm), label: [sì], dy: -7pt)
  #node(12.9cm, 5.6cm, 6.4cm, 1.3cm, [prefisso valido + tasto di aiuto (`help-key`,\ `C-h`) → `(describe-prefix-keys P)`;\ altrimenti «X is undefined» nell'area messaggi])
  #carrow((4.6cm, 5.85cm), (4.6cm, 6.3cm), label: [no], dx: 10pt)
  #node(4.6cm, 6.75cm, 6.4cm, 0.9cm, [una `Lambda`? senza parametri → `(funcall λ)`;\ con parametri → log e fine])
  #carrow((4.6cm, 7.2cm), (4.6cm, 7.65cm))
  #node(4.6cm, 8.1cm, 6.4cm, 0.9cm, [`run_command_form(ast)`], fill: paper)
  #note(0.2cm, 8.9cm)[I tasti vengono registrati per la macro di tastiera in corso anche quando non eseguono nulla:\ una macro deve riprodurre ciò che è stato premuto, compresi `C-u 5` e i tasti non associati.]
  #note(0.2cm, 9.9cm)[Il front-end chiama `handle_key_event` solo per gli eventi di pressione; l'incolla arriva da\ `handle_paste` come un unico comando `(insert-pasted-text TESTO)`, che passa anch'esso da `run_command_form`.]
]

In codice, la funzione è una sequenza di uscite anticipate:

#estratto("editor/keys.rs")[
```rust
pub fn handle_key_event(&self, event: KeyEvent, env: &Arc<Env<EditorState<B>>>) {
    if let Some(watcher) = env.get_variable(KEY_CAPTURE_FUNCTION) && watcher.is_truthy() {
        self.record_keys(&[event.clone()], None);
        self.capture_key_sequence(event, watcher, env);     // read-key-sequence
        return;
    }
    if !(Commands::is_prefix_argument_key(&event) && self.local_key_claims(&event))
        && self.read_prefix_argument(&event)                // C-u, cifre, -
    {
        self.record_keys(&[event], None);
        return;
    }
    let mut sequence = self.pending_key_events();
    sequence.push(event.clone());
    let mut ast = match self.resolve_key_sequence(event.clone()) {
        Dispatch::Run(ast) => { self.record_keys(&sequence, command_named_by(&ast)); ast }
        Dispatch::Nothing  => return,                       // un prefisso: si aspetta
        Dispatch::Unbound { described, prefix, last } => {
            /* prefisso + tasto di aiuto → describe-prefix-keys; altrimenti
               "X is undefined" nell'area messaggi */
            return;
        }
    };
    if let ELispExp::Lambda(ref lambda) = ast {             // un tasto legato a una lambda
        if !lambda.params.is_empty() { /* log */ return; }
        ast = ELispExp::form(vec![ELispExp::symbol("funcall"), ast]);
    }
    self.run_command_form(ast, env);
}
```
]

== L'argomento prefisso

`C-u` e le cifre non arrivano alle keymap: `Commands::read_prefix_argument` li
prende per primo, a meno che la keymap locale del buffer (`set-local-keymap`)
non leghi `C-u` lei stessa (`local_key_claims`). Lo stato è la coppia
`prefix_arg: Option<PrefixArg>` e `reading_prefix: bool`.

#canvas(height: 3.7cm)[
  #node(1.5cm, 1.7cm, 2.4cm, 0.8cm, [nessuno], fill: paper)
  #node(5.8cm, 1.7cm, 2.8cm, 0.8cm, [`Raw(n)`: 4#super[n]])
  #node(10.8cm, 0.5cm, 3.0cm, 0.7cm, [`Negative`: −1])
  #node(10.8cm, 2.9cm, 3.0cm, 0.7cm, [`Number(k)`])
  #carrow((2.7cm, 1.7cm), (4.4cm, 1.7cm), label: [`C-u`], dy: -7pt)
  #note(4.75cm, 0.85cm)[`C-u` di nuovo: n+1]
  #carrow((7.2cm, 1.5cm), (9.3cm, 0.65cm), label: [`-`], dy: -7pt)
  #carrow((7.2cm, 1.9cm), (9.3cm, 2.75cm), label: [cifra], dy: 8pt)
  #carrow((10.8cm, 0.85cm), (10.8cm, 2.55cm), label: [cifra], dx: 14pt)
  #note(12.45cm, 2.8cm)[cifra: k·10 ± c]
  #note(12.6cm, 1.35cm)[da ogni stato, un altro tasto:\ il comando parte con l'argomento]
]

Un tasto qualunque che non fa parte dell'argomento chiude la lettura
(`reading_prefix = false`) ma lascia l'argomento al comando che segue. Il
comando lo legge tramite `call-interactively` (le specifiche `p` e `P`); dopo
#emph[ogni] comando, riuscito o fallito, `clear_prefix_argument` lo cancella.
Non esistono le forme `M-4` o `C-4` di Emacs. Mentre l'argomento e la sequenza
sono a metà, `pending_input()` li descrive (`C-u 4 C-x-`) e la TUI li mostra a
destra nella riga dei messaggi.

I caratteri sono legati a forme che portano già il loro argomento,
`(self-insert "x")`; una forma così non passa da `call-interactively`, e quindi
non riceve l'argomento prefisso: `C-u 3 x` inserisce una sola «x»
#rapporto("CMD-5").

== La risoluzione: `resolve_key_sequence`

Il tasto si aggiunge a `pending_keys`; poi, con un solo lock di lettura su
`modes`, la sequenza viene cercata in quattro keymap, in ordine:

+ la #emph[keymap transitoria], se installata;
+ la keymap locale del buffer corrente, se `set-local-keymap` gliene ha data
  una;
+ la keymap del modo maggiore del buffer corrente;
+ la keymap globale.

#estratto("editor/keys.rs")[
```rust
let mut pending = self.pending_keys.write().unwrap();
pending.push(event);
let names = self.keymap_modes();     // la keymap locale, se c'è, e il modo
let current_modes: Vec<&str> = names.iter().map(String::as_ref).collect();
let bound = self.modes(|modes| {
    if let Some(map) = modes.transient() {
        match (
            map.keymap.get(&pending).cloned(),
            map.keymap.is_prefix(&pending),
            map.on_unbound,
        ) {
            (Some(ast), _, _) => return Bound::Command(ast),
            (None, true, _)   => return Bound::Prefix,
            (None, false, OnUnbound::Refuse)  => return Bound::Refused,
            (None, false, OnUnbound::Pass)    => {}
            (None, false, OnUnbound::Release) => release_transient = true,
        }
    }
    // la keymap locale, poi il modo, poi la keymap globale
    let hit = modes.binding_below_transient(&current_modes, &pending);
    let prefix = modes.is_prefix_below_transient(&current_modes, &pending);
    match (hit, prefix) {
        (Some(binding), _) => Bound::Command(binding.target),
        (None, true)  => Bound::Prefix,
        (None, false) => Bound::Unbound,
    }
});
if !matches!(bound, Bound::Prefix) { pending.clear(); }
```
]

Una sequenza è cercata #emph[intera]: `pending_keys` contiene già i tasti
precedenti, e la tabella di una keymap associa sequenze di tasti, non tasti
singoli. Un risultato `Prefix` lascia la sequenza in `pending_keys`, in attesa
del tasto successivo; ogni altro risultato la svuota. Un modo che lega un
prefisso tiene viva la sequenza anche se solo la globale la
completa. Le stesse due funzioni (`binding_below_transient`,
`is_prefix_below_transient`) rispondono anche a `key-binding` e ai comandi di
aiuto, così ciò che l'aiuto riporta e ciò che l'editor esegue non possono
divergere.

La keymap transitoria (`TransientKeymap`) è installata da una primitiva o dal
meccanismo di ripetizione e dice che cosa fare di un tasto che non conosce:

#ref-table(
  columns: (2.4cm, 1fr, 1fr),
  header: ([`OnUnbound`], [Comportamento], [Uso tipico]),
  [`Refuse`], [Il tasto è rifiutato in silenzio; la mappa resta. Il messaggio della mappa occupa la riga dei messaggi.], [Una domanda (`y-or-n`, `save-some-buffers`): si risponde solo con i suoi tasti.],
  [`Pass`], [Il tasto prosegue verso le altre keymap; la mappa resta.], [La striscia dei completamenti: si può continuare a scrivere.],
  [`Release`], [Il tasto prosegue; la mappa viene tolta.], [La ripetizione (`C-x o o o`): un'offerta che si può ignorare. Anche la vista predefinita di `query-replace` #rapporto("RIC-8").],
)

Dopo ogni comando, `install_repeat_keymap` controlla se il comando ha un
#emph[tasto di ripetizione] dichiarato (`define-repeat-key`) e in tal caso
installa una mappa `Release` che lo associa al comando stesso.

== `run_command_form`: l'esecuzione di un comando <esecuzione>

Tutto ciò che il tasto, l'incolla, il mouse, la cattura e la riproduzione di
macro fanno eseguire passa da qui.

#canvas(height: 10.9cm)[
  #lane(1.5cm, 0cm, 10.8cm, [`run_command_form`], w: 2.9cm)
  #lane(5.3cm, 0cm, 10.8cm, [undo / kill])
  #lane(8.6cm, 0cm, 10.8cm, [`eval`])
  #lane(11.7cm, 0cm, 10.8cm, [`Commands`])
  #lane(14.6cm, 0cm, 10.8cm, [`Modes`])

  #msg(1.5cm, 5.3cm, 1.2cm, [`undo::begin_command(self-insert?)`])
  #msg(1.5cm, 5.3cm, 1.7cm, [`roll_over_command_flags`])
  #seq-note(1.65cm, 1.9cm)[forma senza argomenti `(nome)` o `nome`\ → `(call-interactively "nome")`]
  #msg(1.5cm, 14.6cm, 3.2cm, [`begin_command()`; `hooks_for(modo, "pre-command-hook")`])
  #msg(1.5cm, 8.6cm, 3.7cm, [ogni hook, poi `eval(ast)`, nello stesso scope])
  #msg(8.6cm, 1.5cm, 4.15cm, [`Ok` / `Err`], dash: "dashed")
  #msg(1.5cm, 11.7cm, 4.6cm, [`clear_prefix_argument`])
  #seq-note(1.65cm, 4.8cm)[`Err` → `report_error` → `(report-error MSG FRAMES)`; fine]
  #msg(1.5cm, 14.6cm, 5.8cm, [`hooks_for(modo, "post-self-insert-hook")`])
  #msg(1.5cm, 8.6cm, 6.3cm, [ogni hook, con il suo scope], dash: none)
  #msg(1.5cm, 14.6cm, 6.9cm, [`hooks_for(modo, "post-command-hook")`])
  #msg(1.5cm, 8.6cm, 7.4cm, [ogni hook, con il suo scope])
  #msg(1.5cm, 11.7cm, 8.0cm, [`set_last_form(ast)` (non per `repeat`)])
  #msg(1.5cm, 14.6cm, 8.6cm, [`install_repeat_keymap`])
  #msg(1.5cm, 11.7cm, 9.2cm, [`set_last_command(nome)`])
  #seq-note(1.65cm, 9.5cm)[`post-self-insert-hook` solo se il comando è `self-insert`]
]

#estratto("editor/commands.rs")[
```rust
pub(super) fn run_command_form(&self, mut ast: ELispExp<B>, env: &Arc<Env<EditorState<B>>>) {
    let bound_command = /* Some(nome) se ast è `nome` o `(nome)`, cioè senza argomenti */;
    let this_command  = /* il simbolo in testa, se c'è */;
    undo::begin_command(this_command == Some("self-insert"));     // nuova epoca
    self.roll_over_command_flags();                                // kill ring
    if let Some(name) = bound_command {
        ast = ELispExp::form(vec![
            ELispExp::symbol("call-interactively"), ELispExp::string(name),
        ]);
    }
    let outcome = {
        let _command = self.begin_command();
        let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
        self.run_hook(&mode, "pre-command-hook", env);
        eval(&ast, env.clone(), self)
    };
    self.clear_prefix_argument();
    if let Err(e) = outcome {
        self.report_error(&format!("{:?} {:?}", ast, e), env); return;
    }
    let mode = self.with_current_buffer(|buf| buf.current_mode.clone());
    if this_command == Some("self-insert") {
        self.run_hook(&mode, "post-self-insert-hook", env);
    }
    self.run_hook(&mode, "post-command-hook", env);
    if this_command != Some("repeat") { self.commands_mut(|c| c.set_last_form(Some(ast))); }
    self.install_repeat_keymap(this_command);
    self.set_last_command(this_command);
}
```
]

+ #emph[Undo.] `undo::begin_command` assegna al thread una nuova epoca; tutte
  le modifiche del comando finiranno in un solo gruppo (@undo).
+ #emph[Flag del kill ring.] «Il comando precedente ha ucciso» diventa vero o
  falso #emph[prima] che il comando parta, così una serie di `C-k` si accumula
  in una sola voce del kill ring e un comando che fallisce non lascia il flag
  acceso.
+ #emph[Instradamento.] Una forma senza argomenti diventa una chiamata a
  `call-interactively` con il nome del comando, così l'editor raccoglie gli
  argomenti dichiarati. Una
  forma che porta già i suoi argomenti --- `(self-insert "a")` --- resta
  com'è: il percorso della digitazione non paga nulla.
+ #emph[Esecuzione.] `pre-command-hook`, poi `eval`, dentro lo stesso scope di
  carburante: l'hook spende il budget del comando.
+ #emph[Argomento prefisso] cancellato, anche se il comando è fallito.
+ #emph[Errore.] `report_error` incrementa `command_errors`, prende la pila
  delle chiamate come backtrace, la svuota e chiama la funzione Lisp
  `report-error` (definita in `debug.lisp`: messaggio nell'area messaggi e, se
  `debug-on-error`, una finestra `*Backtrace*`). Senza quella funzione scrive
  nel log. Dopo un errore non girano gli hook e `last-command` non cambia.
+ #emph[Hook.] Prima `post-self-insert-hook` (solo per `self-insert`: qui si
  aggancia `electric-pair`), poi `post-command-hook`. Ogni lista è la
  concatenazione degli hook del modo e di quelli globali, copiata con un solo
  lock; un hook che fallisce viene registrato nel log e non ferma gli altri.
+ #emph[Memoria.] La forma eseguita diventa quella di `repeat`; si offre il
  tasto di ripetizione; `last-command` riceve il nome.

== La cattura di una sequenza <cattura>

`read-key-sequence` arma `*key-capture-function*`. Il tasto successivo segue la
stessa risoluzione di un tasto normale --- la sequenza a metà si vede come
sempre --- ma al posto di eseguire il comando viene chiamata la funzione con la
sequenza, la forma che sarebbe stata eseguita (citata) e il nome della keymap
che la fornisce, o `"prefix-argument"` per `C-u`. È così che `describe-key` e
`zap-to-char` leggono un tasto: non esiste in rsedit un «leggi un tasto» che
blocchi, perché i tasti arrivano uno per giro del ciclo degli eventi.

== Le decisioni <decisioni-tasto>

+ *Un percorso solo esegue un comando.* Il tasto, l'incolla, il mouse, la
  cattura e la macro passano tutti da `run_command_form`: un secondo percorso
  andrebbe tenuto in passo con il primo a mano, per l'undo, gli hook e
  `last-command`.
+ *Un tasto che allunga una sequenza non è un comando.* `resolve_key_sequence`
  risponde invece di eseguire. `C-x` non deve chiudere il gruppo di undo in cui
  si sta scrivendo, né spezzare in due voci una serie di `C-k`: sono cose che
  succedono tutte dopo il punto in cui la risoluzione restituisce.
+ *La sequenza a metà sta nell'editor, non in un modo.* Una keymap di modo si
  consulta prima della globale, e una sequenza cominciata sotto l'una non deve
  restare a metà nell'altra. `pending_keys` è un vettore che si riempie e si
  svuota, così il tasto che è da solo un legame non alloca nulla.
+ *La cattura segue la risoluzione vera.* `read-key-sequence` non intercetta i
  tasti prima della risoluzione: li accumula nella stessa sequenza, così un
  `C-x` a metà si vede come sempre, e devia solo al posto dell'esecuzione. Ciò
  che fa un tasto ha una sola risposta vera, quella su cui l'editor agirebbe;
  una seconda implementazione delle precedenze riporterebbe quella vecchia il
  giorno in cui l'ordine cambia.
+ *Armare la cattura è una variabile.* `*key-capture-function*` è un fatto in un
  posto solo, che il Lisp legge e imposta come ogni variabile, al prezzo di una
  ricerca per tasto; nil quando nessuno cattura. Si disarma prima di chiamare
  la funzione: una funzione che apre un buffer o fallisce non deve lasciare
  catturato anche il tasto dopo.
+ *Il tasto di aiuto dopo un prefisso è una domanda.* Una sequenza non
  associata, il cui ultimo tasto è l'ultimo di `help-key` e il cui resto è un
  prefisso vivo, chiama `describe-prefix-keys`; senza un modulo di aiuto è
  indefinita come ogni altra. La variabile si legge solo su una sequenza già
  non associata, mai su un tasto digitato.
+ *Un incolla è un comando solo.* Il terminale dice che un incolla è un
  incolla. Digitato tasto per tasto sarebbe annullato un carattere alla volta,
  e `electric-pair` accoppierebbe di nuovo ogni parentesi: arriva invece come
  `insert-pasted-text`, con un gruppo di undo, un `post-command-hook` e nessun
  `post-self-insert-hook`.
+ *Dopo un comando senza tasto di ripetizione non si toglie nulla.* Una mappa
  `Release` l'ha già tolta la risoluzione, passando il tasto oltre; una
  `Refuse`, una domanda, risponde con i suoi comandi, e `install_repeat_keymap`
  gira dopo ognuno: toglierla lì la toglierebbe appena ha avuto risposta.

// ===========================================================================
= Comandi e argomenti <comandi>
// ===========================================================================

== Che cos'è un comando

Per l'interprete un comando non esiste: è una funzione qualunque. Diventa un
comando quando ha una voce nel registro di `Commands` (`register_command`), che
associa al nome l'elenco degli argomenti da chiedere. Una primitiva diventa
comando registrandosi con `into.command(nome, puntatore, specifiche, DOC)`
invece che con `into.function`; una funzione Lisp con la macro `defcommand` di
`commands.lisp`:

```lisp
(defcommand greet (who) ("sGreet whom: ")
  "Say hello to WHO."
  (message "Hello, %s" who))
```

`M-x` completa solo sui nomi registrati; una funzione non registrata si può
comunque legare a un tasto, e viene chiamata senza argomenti. Il registro,
`CommandRegistry`, è una tabella dal nome del comando alle sue specifiche, già
analizzate (`ArgSpec::parse`).

== Le specifiche degli argomenti

#ref-table(
  columns: (1.6cm, 2.6cm, 1fr),
  header: ([Codice], [`ArgSpec`], [Valore passato al comando]),
  [`s`], [`String`], [Il testo digitato nel prompt.],
  [`n`], [`Number`], [Il testo convertito in numero; un testo non numerico diventa 0 #rapporto("CMD-6").],
  [`b`], [`Buffer`], [Un nome di buffer, con completamento sui buffer vivi (escluso `*Minibuffer*`).],
  [`f`], [`File`], [Un percorso, completato un componente alla volta.],
  [`p`], [`Count`], [L'argomento prefisso come numero, 1 se assente. Non chiede nulla.],
  [`P`], [`RawCount`], [L'argomento grezzo: `nil`, `(4)` per `C-u`, un numero, o il simbolo `-`. Non chiede nulla.],
  [`r`], [`Region`], [#emph[Due] valori, inizio e fine della regione attiva. Senza regione il comando è rifiutato prima di aprire qualunque prompt.],
)

== `call-interactively` e la catena degli argomenti

Il minibuffer non blocca: apre un prompt e ritorna, e la risposta arriva con un
tasto successivo. Raccogliere due argomenti non può quindi essere un ciclo: è
una catena, e lo stato tra un anello e l'altro sta nella pila dei
`PendingCommand` di `Commands`.

#canvas(height: 10.6cm)[
  #lane(1.5cm, 0cm, 10.5cm, [tasto `C-x C-f`], fill: accent.lighten(85%), w: 2.8cm)
  #lane(5.4cm, 0cm, 10.5cm, [`call-interactively`], w: 3.2cm)
  #lane(9.0cm, 0cm, 10.5cm, [`Commands`])
  #lane(12.0cm, 0cm, 10.5cm, [`minibuffer-read`], w: 2.8cm)
  #lane(14.9cm, 0cm, 10.5cm, [`find-file`])

  #msg(1.5cm, 5.4cm, 1.2cm, [`(call-interactively "find-file")`])
  #msg(5.4cm, 9.0cm, 1.65cm, [`command_specs` → `["fFind file: "]`])
  #msg(5.4cm, 9.0cm, 2.1cm, [`capture_invocation()` + `push_pending`])
  #msg(5.4cm, 9.0cm, 2.55cm, [`fill_answerable_args` → `File`])
  #msg(5.4cm, 12.0cm, 3.05cm, [titolo, confirm, complete, cancel])
  #msg(12.0cm, 5.4cm, 3.5cm, [prompt aperto: ritorna], dash: "dashed")
  #msg(5.4cm, 1.5cm, 3.95cm, [fine del comando], dash: "dashed")
  #seq-note(1.65cm, 4.25cm)[l'utente scrive il percorso: ogni carattere è un `self-insert`\ nel buffer `*Minibuffer*`; Tab chiama la funzione di completamento]
  #msg(1.5cm, 12.0cm, 5.55cm, [Invio → `minibuffer-confirm`])
  #msg(12.0cm, 5.4cm, 6.05cm, [`command_arg_confirm(testo)`])
  #msg(5.4cm, 9.0cm, 6.5cm, [`accept_pending_arg`])
  #msg(5.4cm, 9.0cm, 6.95cm, [`fill_answerable_args` → nessuno])
  #msg(5.4cm, 9.0cm, 7.4cm, [`take_pending` → `("find-file", [percorso])`])
  #msg(5.4cm, 14.9cm, 7.9cm, [`call_callable(find-file, [percorso])`])
  #msg(14.9cm, 5.4cm, 8.4cm, [valore], dash: "dashed")
  #seq-note(5.55cm, 8.7cm)[con più argomenti, al posto della chiamata\ si apre il prompt successivo: stessa funzione `advance_pending`]
]

`Invocation` --- argomento prefisso e regione --- è catturata quando il comando
inizia (`capture_invocation`), non quando l'ultimo argomento arriva, e risponde
agli argomenti `p`, `P` e `r` di tutta la catena.

Il titolo del prompt dice chi sta chiedendo: `find-file - Find file:` o, con più
argomenti, `query-replace (2/2) - With:`. Il prompt viene aperto chiamando
`minibuffer-read` per nome, così un minibuffer alternativo serve anche gli
argomenti dei comandi. Annullare il prompt (`command_arg_cancel`) toglie il
comando dalla pila: un elenco di argomenti incompleto non arriva mai a un
comando. Un nuovo `call-interactively` senza minibuffer aperto svuota la pila
dagli orfani rimasti da prompt chiusi in altro modo.

Il completamento degli argomenti `b` e `f` è fatto in Rust; una funzione in
`*command-arg-completion-function*`, se impostata, lo sostituisce per tutti i tipi
ricevendo il tipo come simbolo (`buffer`, `file`, …) e il prefisso.

== Le decisioni <decisioni-comandi>

+ *Un comando è un'idea dell'editor, non dell'interprete.* Un comando è
  qualcosa che si raggiunge con `M-x` o con un tasto, e di cui l'editor sa
  chiedere gli argomenti: un nome di file con il completamento, un buffer, un
  numero. Per un interprete che non sa di stare in un editor non significa
  nulla. Così l'interprete non ne sa niente: un comando è una funzione
  qualunque con una voce nel registro, `(find-file "/tmp/x")` da Lisp è una
  chiamata come le altre, e né il tipo `Lambda` né `eval` devono sapere che i
  comandi esistono.
+ *Le specifiche si analizzano alla registrazione.* Un codice sbagliato, come
  `"zFind file: "`, diventa un errore all'avvio dell'editor invece di una
  sorpresa la prima volta che qualcuno esegue il comando. Per la stessa
  ragione un testo dopo `p`, `P` o `r`, che non chiedono nulla, è un errore e
  non un dettaglio da ignorare.
+ *Prefisso e regione si fissano quando il comando comincia.* Gli argomenti
  non arrivano tutti insieme: uno chiesto arriva qualche tasto dopo, da un
  minibuffer che per aprirsi ha cambiato buffer. Quando arriva l'ultimo, la
  regione appartiene a un altro buffer e l'argomento prefisso è già stato
  cancellato per il comando successivo. All'inizio sono invece ciò che
  l'utente intendeva: `C-u 4 M-x foo` passa quattro a `foo`, per quanto a
  lungo resti aperto il prompt.
+ *I comandi in attesa sono una pila.* Un comando può cominciare mentre un
  altro sta ancora chiedendo i suoi argomenti (`M-x` dentro un prompt), e i
  due devono chiudersi nell'ordine inverso a quello in cui sono cominciati. Un
  posto solo perderebbe il primo.

// ===========================================================================
= Minibuffer e ricerca incrementale <minibuffer>
// ===========================================================================

== Il minibuffer

Il meccanismo del minibuffer è in Rust (`feature/minibuffer.rs`,
`primitives/minibuffer.rs`) perché troppe cose ne dipendono --- `M-x`, `M-:`,
gli argomenti dei comandi --- per affidarlo a un file Lisp che potrebbe
mancare. L'aspetto, invece, è sostituibile: `minibuffer-read` chiama la
funzione contenuta in `*minibuffer-read-function*`, che per impostazione
predefinita è `default-minibuffer-prompt`.

```lisp
(minibuffer-read PROMPT ON-CONFIRM ON-CHANGE ON-CANCEL [MODE [HISTORY-KEY]])
```

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Argomento], [Significato]),
  [`ON-CONFIRM`], [Chiamata con il testo finale quando si preme Invio, #emph[dopo] la chiusura del prompt.],
  [`ON-CHANGE`], [Nonostante il nome, è la funzione di completamento: riceve il testo e restituisce i candidati. È chiamata al primo Tab dopo ogni modifica; i Tab successivi scorrono i candidati, o li consegnano a `*completion-read-function*` se impostata (è ciò che fa `completion.lisp`).],
  [`ON-CANCEL`], [Chiamata senza argomenti quando si preme Esc.],
  [`MODE`], [Il modo del buffer del prompt (predefinito `minibuffer-mode`); `isearch-mode` per la ricerca.],
  [`HISTORY-KEY`], [La chiave della storia (`M-p`, `M-n`); per default il testo del prompt, così ogni prompt ha una storia senza chiederla. Fino a `history-length` voci, 100 per default.],
)

`default-minibuffer-prompt` salva le callback in variabili globali
(`*minibuffer-on-confirm*`, `*minibuffer-on-change*`, `*minibuffer-on-cancel*`,
`*minibuffer-previous-buffer*`, …) e apre una finestra flottante con bordo sul
buffer `*Minibuffer*`, #emph[al centro] del frame, larga `minibuffer-width` (60)
e alta `minibuffer-height` (3) e ridotta se il terminale è più piccolo. Il
centro è scelto perché il fondo dello schermo è occupato da altre strisce (i
completamenti, la compilazione); alcuni commenti descrivono ancora un prompt
agganciato in basso #rapporto("DOC-1"). Ogni prompt usa lo stesso buffer,
`*Minibuffer*`: due prompt aperti insieme lo condividono #rapporto("MIN-1").
`C-g` nel prompt resta `keyboard-quit`, che non lo chiude; lo chiude Esc
#rapporto("MIN-2").

Un worker di sfondo non può aprire un prompt: `minibuffer-read` controlla
`in_worker()` e rifiuta con un errore, perché il tasto che risponderebbe
arriverebbe in mezzo a ciò che l'utente sta facendo.

== La ricerca incrementale

`C-s` apre un prompt in `isearch-mode` e ricorda dove era il punto. Da lì il
lavoro lo fa il `post-command-hook` di quel modo: ogni comando eseguito mentre
il prompt è aperto --- ogni carattere scritto, ogni cancellazione --- provoca
un giro di `isearch-update`, che ripete la ricerca e sposta il punto. Non serve
un ciclo «leggi un tasto», che in rsedit non esiste.

#key[
  Mentre il prompt è aperto il buffer corrente è il minibuffer. Tutto ciò che
  riguarda la ricerca lavora quindi sul buffer cercato #emph[per nome], preso
  dallo stato della sessione (`Isearch`) in `Runtime`. Una modifica che usasse il
  buffer corrente cercherebbe nella riga del prompt; il test
  `an_isearch_searches_the_file_not_the_prompt` esiste per questo. Lo stato della
  sessione e l'algoritmo di ricerca sono descritti nel @testo.
]

// ===========================================================================
= I buffer <buffer>
// ===========================================================================

== La struttura `Buffer`

#ref-table(
  columns: (3.2cm, 1fr),
  header: ([Campo], [Contenuto]),
  [`name`, `file_path`], [Nome e file visitato.],
  [`text: B`], [Il testo, tramite `BufferTrait` (oggi `GapBuffer`). Contiene anche il punto (il cursore).],
  [`is_modified`, `read_only`], [Modificato rispetto al disco; rifiuta le modifiche (controllato alle due porte).],
  [`current_mode`], [Il nome del modo maggiore; decide keymap, hook, grammatica e tabella sintattica.],
  [`undo`], [`UndoHistory`: gruppi fatti e disfatti (@undo).],
  [`mark: Option<Mark>`], [Posizione e flag `active`; la regione è tra mark attivo e punto.],
  [`overlays`], [`OverlayTable`: facce su intervalli, per categoria, che seguono le modifiche.],
  [`virtual_text`], [Testo mostrato in una posizione senza essere nel buffer (suggerimenti, anteprime).],
  [`version`], [Incrementata da ogni modifica: un risultato calcolato su un'altra versione viene rifiutato.],
  [`changed`], [L'argomento emesso a ogni modifica del testo: il `buffer-changed` dell'editor, da quando il buffer vi entra (@argomenti).],
  [`syntax`, `scan`], [Cache della colorazione e della scansione delle espressioni.],
  [`file_stamp`, `stale`], [Timbro (istante di modifica e lunghezza) del file all'ultima lettura o scrittura; flag «cambiato sotto di noi».],
  [`auto_saved_at`], [Versione scritta nell'ultimo salvataggio automatico.],
  [`data`], [`HashMap<String, Box<dyn Any>>`: dati che altre parti appendono al buffer e che muoiono con lui.],
  [`writer`], [Il lavoro che scrive nel buffer da un thread suo (un comando di shell, una ricerca); chiudere il buffer lo ferma.],
)

== `BufferTrait` e `GapBuffer` <gap-buffer>

`BufferTrait` è l'unica cosa che il resto dell'editor può assumere sul testo:
lunghezza, carattere in una posizione, punto in coordinate lineari e in
(riga, colonna), conversioni tra le due, spostamenti del punto, inserimento e
cancellazione di un carattere #emph[al punto], righe, `clear`. Gli
spostamenti e le conversioni lavorano in caratteri, non in byte. Ciò che ogni
implementazione deve fare è scritto una volta sola, per un `B` qualunque, in
`buffer_conformance_tests`: una seconda implementazione è una riga in fondo a
quel file, e arriva già provata. Il trait chiede anche `Clone`, ma il `clone`
di `GapBuffer` è un `todo!()` #rapporto("BUF-3").

`GapBuffer` tiene un `Vec<char>` con un buco al punto: inserire e cancellare
lì costa O(1), spostare il punto costa la distanza percorsa. Accanto al testo
tiene un indice delle posizioni fisiche dei `\n`, diviso in prima e dopo il
buco: poiché le posizioni fisiche non cambiano quando il buco cresce o si
restringe, scrivere un carattere aggiorna l'indice in O(1).

#estratto("buffer/gap_buffer.rs")[
```rust
pub struct GapBuffer {
    data: Vec<char>,                 // testo e buco nello stesso vettore
    gap_start: usize,                // primo indice del buco = posizione del punto
    gap_end: usize,                  // primo indice dopo il buco
    newlines_before: Vec<usize>,     // posizioni fisiche dei \n prima del buco
    newlines_after: VecDeque<usize>, // posizioni fisiche dei \n dopo il buco
}
fn insert(&mut self, c: char) {
    if self.gap_start == self.gap_end { self.gap_grow(); }   // buco esaurito: raddoppia
    self.data[self.gap_start] = c;
    if c == '\n' { self.newlines_before.push(self.gap_start); }
    self.gap_start += 1;
}
```
]

Spostare il punto vuol dire spostare il buco: `move_gap` copia con
`copy_within` i caratteri che il buco deve scavalcare dall'altra parte, e
sposta tra le due metà dell'indice i ritorni a capo che attraversano il buco,
correggendone la posizione fisica della larghezza del buco. Un ritorno a capo
non cambia mai posizione fisica per un inserimento o una cancellazione al
punto, ed è questo che rende O(1) l'aggiornamento dell'indice durante la
digitazione. La posizione #emph[logica] del k-esimo ritorno a capo è
`newlines_before[k]` se k cade nella prima metà, altrimenti la posizione fisica
nella seconda metà meno la larghezza del buco; `line_start(riga)` è la
posizione del ritorno a capo precedente più uno. La conversione da posizione
lineare a (riga, colonna) del punto, `cursor_pos`, conta invece i caratteri
dall'inizio del buffer #rapporto("BUF-2").

#canvas(height: 2.6cm)[
  #node(1.2cm, 0.8cm, 1.6cm, 0.7cm, [`d a t`], fill: paper)
  #node(3.6cm, 0.8cm, 3.2cm, 0.7cm, [buco (gap)], stroke-colour: dim)
  #node(6.2cm, 0.8cm, 2.0cm, 0.7cm, [`i \n x`], fill: paper)
  #note(3.0cm, 1.3cm)[`gap_start` = punto]
  #note(10.0cm, 0.3cm)[`newlines_before: Vec<usize>`]
  #note(10.0cm, 0.8cm)[`newlines_after: VecDeque<usize>` (posizioni fisiche)]
  #note(0.2cm, 1.9cm)[un carattere costa 4 byte (`char`); il buco cresce raddoppiando quando si esaurisce]
]

== Le due porte <due-porte>

Ogni primitiva che cambia il testo passa da `edits::insert_text` o da
`edits::delete_range`. È la proprietà su cui si reggono undo, overlay, cache e
sola lettura: una nuova primitiva di modifica li ottiene tutti per il solo
fatto di non poter evitare le porte.

#canvas(height: 6.4cm)[
  #node(3.3cm, 0.45cm, 6.2cm, 0.7cm, [`insert_text(buf, at, s)` / `delete_range(buf, a, b)`])
  #carrow((3.3cm, 0.8cm), (3.3cm, 1.2cm))
  #node(3.3cm, 1.6cm, 6.2cm, 0.7cm, [`read_only`? → rifiuta (`false`)], fill: warm.lighten(85%))
  #carrow((3.3cm, 1.95cm), (3.3cm, 2.35cm))
  #node(3.3cm, 2.75cm, 6.2cm, 0.7cm, [`undo.record_insert` / `record_delete` (con il testo tolto)])
  #carrow((3.3cm, 3.1cm), (3.3cm, 3.5cm))
  #node(3.3cm, 3.9cm, 6.2cm, 0.7cm, [mark disattivato; overlay e testo virtuale adattati])
  #carrow((3.3cm, 4.25cm), (3.3cm, 4.65cm))
  #node(3.3cm, 5.05cm, 6.2cm, 0.7cm, [modifica del testo; `is_modified = true`])
  #carrow((6.4cm, 5.05cm), (8.6cm, 5.05cm))
  #node(12.3cm, 5.05cm, 7.2cm, 1.4cm, [`changed(buf, at)`:\ `version += 1`;\ `syntax.invalidate_from(riga)`, `truncate`;\ `scan.invalidate_from(riga)`; `changed.emit()`], fill: paper)
  #note(8.6cm, 1.0cm)[Entrambe restituiscono `bool` con `#[must_use]`:\ chi chiama deve dire all'utente se il buffer ha rifiutato\ (`"Buffer is read-only"`).]
  #note(8.6cm, 2.6cm)[Il mark non viene spostato: una modifica lo disattiva,\ e un mark attivo non può quindi aver subito modifiche.\ La posizione resta per `exchange-point-and-mark`.]
]

#estratto("primitives/edits.rs")[
```rust
pub(crate) fn insert_text<B: BufferTrait>(
    buf: &mut Buffer<B>, at: usize, content: &str,
) -> bool {
    if buf.read_only { return false; }
    if content.is_empty() { return true; }
    let at = at.min(buf.text.len());
    let point = buf.text.cursor_pos_1d();
    let inserted = content.chars().count();
    buf.undo.record_insert(at, inserted, point);          // 1. la storia
    buf.mark = deactivated(buf.mark);                     // 2. la regione
    buf.overlays.adjust_for_insert(at, inserted);         // 3. ciò che è ancorato al testo
    buf.virtual_text.adjust_for_insert(at, inserted);
    undo::apply_insert(&mut buf.text, at, content);       // 4. il testo
    buf.is_modified = true;
    changed(buf, at);                                     // 5. versione e cache
    true
}
pub(crate) fn changed<B: BufferTrait>(buf: &mut Buffer<B>, at: usize) {
    buf.version = buf.version.wrapping_add(1);
    let line = buf.text.cursor_1d_to_2d(at.min(buf.text.len())).0;
    buf.syntax.invalidate_from(buf.version, line);
    buf.syntax.truncate(buf.text.line_count());
    buf.scan.invalidate_from(buf.version, line);
    buf.changed.emit();                                   // sveglia chi aspetta
}
```
]

`delete_range` ha la stessa forma. Ordina e limita gli estremi, copia il testo
che sta per togliere (tutto il buffer con `to_string`, una porzione con
`slice`), lo consegna a `record_delete` --- è l'unico modo in cui l'undo potrà
rimetterlo --- e poi adatta overlay e testo virtuale, cancella e chiama
`changed` dalla posizione di partenza. L'ordine conta: la storia registra lo
stato #emph[prima] della modifica, gli overlay vengono spostati con gli stessi
numeri che descrivono la modifica, e la versione cambia per ultima, così
nessun lettore può vedere la versione nuova insieme al testo vecchio (tutto
avviene sotto lo stesso lock di scrittura del buffer). Le porte non scrivono
messaggi: un rifiuto lo riferisce chi le chiama, con `edited`, dopo la closure
del buffer (@primitive-modifica).

Tre percorsi oggi scavalcano le porte:

- `adopt_text` (lettura di un file, `revert-buffer`, ricarica dal watcher)
  sostituisce tutto il testo e fa a mano ciò che le porte farebbero: incrementa
  la versione, invalida entrambe le cache dalla riga 0, svuota overlay, testo
  virtuale, mark e storia dell'undo (conservandone il limite), riporta il punto
  alla stessa riga e colonna;
- `set_minibuffer_content`, usata dallo scorrimento dei completamenti, scrive
  direttamente nel testo del minibuffer e poi chiama `changed`
  #rapporto("MIN-3");
- `undo` e `redo`, che applicano l'inverso di un gruppo direttamente sul testo
  con `UndoHistory::undo(text)` e poi chiamano `changed`: versione e cache sono
  a posto, ma `read_only` non viene controllato e overlay e testo virtuale non
  si spostano #rapporto("BUF-1").

== Undo <undo>

La storia registra #emph[modifiche], non fotografie: un inserimento è
`Inserted { at, len }`, una cancellazione `Deleted { at, text }`. Un gruppo
è l'insieme delle modifiche di un comando, con la posizione del punto prima
della prima modifica. Disfare applica l'inverso del gruppo e produce il
gruppo che lo rifarebbe; rifare è la stessa operazione sull'altra pila.

#canvas(height: 5.6cm)[
  #node(2.6cm, 0.5cm, 4.8cm, 0.8cm, [`run_command_form`], fill: paper)
  #carrow((2.6cm, 0.9cm), (2.6cm, 1.35cm))
  #node(2.6cm, 1.85cm, 4.8cm, 1.0cm, [`begin_command(self-insert?)`\ `CURRENT` = nuova epoca globale])
  #carrow((5.0cm, 1.85cm), (6.9cm, 1.85cm))
  #node(10.6cm, 1.85cm, 7.2cm, 1.2cm, [prima modifica con un'epoca diversa da quella\ del gruppo aperto → `boundary()`: il gruppo va in `done`\ salvo serie di digitazione sotto i 20 caratteri])
  #carrow((10.6cm, 2.45cm), (10.6cm, 3.0cm))
  #node(10.6cm, 3.5cm, 7.2cm, 1.0cm, [`Inserted` adiacente all'ultimo → unito\ (cento tasti = una voce)])
  #node(2.6cm, 4.0cm, 4.8cm, 1.0cm, [`done: Vec<Group>`\ `undone: Vec<Group>`], fill: paper)
  #note(0.2cm, 4.75cm)[una nuova modifica svuota `undone`: nessun albero dei rami]
]

Le due funzioni che decidono i gruppi sono queste:

#estratto("buffer/undo.rs")[
```rust
pub(crate) fn begin_command(amalgamating_kind: bool) {      // chiamata da run_command_form
    if PINNED.get() > 0 { return; }      // dentro una macro: un gruppo solo
    CURRENT.with(|current| {
        let previous = current.get();
        current.set(Command {
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            amalgamating_kind,
            joins_previous: amalgamating_kind && previous.amalgamating_kind,
        });
    });
}

fn push(&mut self, change: Change, point: usize) {            // ogni modifica passa da qui
    self.undone.clear();                                       // niente albero dei rami
    let command = current_command();
    if self.open.is_some() && command.epoch != self.open_epoch
        && !(command.joins_previous && self.open_insert_len() < AMALGAMATION_LIMIT)
    {
        self.boundary();                 // il gruppo aperto va in `done`
    }
    self.open_epoch = command.epoch;
    self.bytes += change.weight();                             // byte di testo cancellato
    let group = self.open.get_or_insert_with(|| Group { changes: Vec::new(), point });
    if let Change::Inserted { at, len } = change
        && let Some(Change::Inserted { at: prev_at, len: prev_len }) =
            group.changes.last_mut()
        && *prev_at + *prev_len == at
    {
        *prev_len += len;                // inserimento contiguo: si allunga
        return;
    }
    group.changes.push(change);
}
```
]

Disfare un gruppo vuol dire applicare le sue modifiche al contrario, dall'ultima
alla prima, e costruire intanto il gruppo che rifarebbe ciò che si è appena
disfatto:

#estratto("buffer/undo.rs")[
```rust
fn apply_inverse<B: BufferTrait>(&mut self, text: &mut B, group: Group) -> Group {
    let mut inverse = Vec::with_capacity(group.changes.len());
    for change in group.changes.iter().rev() {
        match change {
            Change::Inserted { at, len } => {
                // si toglie, ricordando che cosa
                let removed = text.slice(*at, at + len);
                apply_delete(text, *at, at + len);
                inverse.push(Change::Deleted { at: *at, text: removed });
            }
            Change::Deleted { at, text: content } => {
                apply_insert(text, *at, content);              // si rimette il testo salvato
                inverse.push(Change::Inserted { at: *at, len: content.chars().count() });
            }
        }
    }
    Group { changes: inverse, point: /* posizione della prima modifica */ }
}
```
]

`undo` chiude il gruppo aperto, toglie l'ultimo da `done`, lo applica al
contrario e mette l'inverso in `undone`; `redo` fa l'operazione simmetrica.
Restituiscono la posizione in cui riportare il punto.

- #emph[Il confine è pigro.] Chiamare `boundary` a ogni tasto costava un lock
  di scrittura sul buffer anche per i tasti che non modificano nulla (circa 120
  dei 890 ns di un tasto). Ora un comando timbra un'epoca in una variabile del
  thread (`CURRENT`), e il gruppo aperto si chiude alla prima modifica che
  arriva con un'epoca diversa. Le epoche vengono da un contatore globale, così
  due thread non possono confondere le loro.
- #emph[Digitazione.] `joins_previous` è vero se il comando e il precedente sono
  entrambi `self-insert`; la serie continua nello stesso gruppo finché il
  gruppo non ha inserito `AMALGAMATION_LIMIT` (20) caratteri.
- #emph[Macro.] `OneGroup` (RAII, annidabile) blocca il timbro: tutti i
  comandi di una riproduzione finiscono in un gruppo solo.
- #emph[Fuori dai comandi.] Una modifica fatta da Lisp senza un tasto premuto
  usa l'epoca rimasta nel thread, quindi si unisce al gruppo dell'ultimo
  comando; `undo-boundary` chiude il gruppo a mano.
- #emph[Limite.] `DEFAULT_UNDO_LIMIT` è 1 MiB di #emph[testo cancellato] (solo
  le cancellazioni portano testo); oltre, i gruppi più vecchi vengono scartati.
  `set-undo-limit` lo cambia per buffer. Il gruppo più recente non viene mai
  scartato, anche se da solo supera il limite.

== Mark, regione, overlay e testo virtuale <mark-overlay>

Il mark è una posizione con un flag `active`. La regione esiste solo se il mark
è attivo (`region_bounds` la ordina e la limita alla lunghezza del testo), e
ogni modifica la disattiva, come in Emacs. Il flag `rectangle` dice che la
regione si disegna come un rettangolo; il rettangolo tra due posizioni lo
calcola `Rectangle::between_offsets`, in `text/rectangle.rs` (@testo). Gli
#emph[overlay] sono intervalli con una faccia e una categoria (le
corrispondenze di una ricerca, una diagnostica, l'enfasi di una pagina di
manuale); sono, con il testo virtuale, le sole posizioni conservate che
seguono le modifiche: le porte li adattano con `adjust_for_insert` e
`adjust_for_delete`. Nessuna delle due estremità cresce, e un overlay che si
riduce a lunghezza zero viene cancellato.
Il #emph[testo virtuale] è ancorato a un punto, non a un intervallo, ed è per
questo una tabella a parte: il layout delle righe lo inserisce nel testo
mostrato (`ui/layout.rs`), e un clic su di esso viene riportato alla posizione
vera del buffer.

== Le cache del testo <cache-testo>

#ref-table(
  columns: (2.6cm, 1fr, 1fr),
  header: ([Cache], [Che cosa conserva], [Chi la riempie]),
  [`SyntaxCache`], [Gli intervalli colorati di ogni riga e lo stato del lessico (la pila delle regioni aperte) all'inizio di ogni riga, per versione, modo ed edizione delle grammatiche.], [L'evidenziatore, `LINES_PER_TURN` (500) righe per passo, svegliato da `buffer-changed` e `syntax-changed`.],
  [`ScanCache`], [Un punto di ripresa ogni 64 righe per la scansione delle espressioni bilanciate, per versione, modo ed edizione della tabella sintattica.], [Lo scanner preventivo, 500 righe per passo, svegliato dagli stessi argomenti. `forward-sexp`, l'indentazione e `syntax-ppss` la leggono e partono dal punto di ripresa più vicino invece che dall'inizio.],
)

Entrambe sono invalidate dalla riga modificata in giù, dalla stessa chiamata
(`changed`); la colorazione vecchia resta disegnata finché la nuova non la
sostituisce. Una cache che non corrisponde a versione, modo ed edizione
correnti non viene usata: il caso peggiore è una scansione dall'inizio, la
stessa che si farebbe senza cache.

== File su disco <file-su-disco>

`find-file` legge il file, imposta `file_path` e il timbro (`FileStamp`:
istante di modifica e lunghezza, un solo `stat`), e sceglie il modo secondo le
espressioni regolari di `add-auto-mode`. La lettura usa `read_to_string`, che
rifiuta un file non UTF-8; il motivo va nel log #rapporto("FIL-3").
Le primitive sono in `primitives/files.rs`, e `save-some-buffers` e l'uscita,
che salvano con lo stesso `write_buffer`, in `primitives/quit.rs` (@uscire).
Salvare scrive l'intero testo con `std::fs::write` (sul posto, senza file
temporaneo #rapporto("FIL-2")) e aggiorna il timbro, altrimenti ogni
salvataggio sembrerebbe la scrittura di qualcun altro. `save-buffer` scrive
l'esito, riuscito o fallito, nel log e non nell'area messaggi
#rapporto("FIL-1"); `write-file` lo scrive anche nell'area messaggi.

Il #emph[watcher] (`FileWatcher`, ogni `watch-file-interval` secondi, 3 per
default, al massimo 8 file per turno; spento da `(setq watch-files nil)`)
confronta i timbri: un buffer pulito il cui file è cambiato viene ricaricato con
`adopt_text`; uno modificato, o il cui file è sparito, viene marcato `stale`, e
il salvataggio successivo chiede conferma. L'#emph[autosalvataggio]
(`AutoSaver`, ogni `auto-save-interval`, 30 secondi per default, al massimo 4
buffer per turno; spento da `(setq auto-save nil)`) scrive i buffer modificati
la cui versione è cambiata dall'ultima copia in `#nome#`, nella directory del
file o in `auto-save-directory`; `recover-file` riporta la copia nel buffer.
La versione salvata viene registrata anche quando la scrittura fallisce
#rapporto("FIL-5").

== Le decisioni <decisioni-buffer>

+ *La sola lettura sta sul buffer e si controlla alle porte.* Un buffer che
  #emph[presenta] qualcosa --- un elenco di directory, l'elenco delle facce,
  un backtrace --- è la vista di uno stato che sta altrove: scriverci non
  cambia quello stato, fa solo dire allo schermo una cosa falsa. I tasti da
  soli non bastano a proteggerlo: ogni tasto stampabile è legato a
  `self-insert`, e `M-x kill-line` o una riga di Lisp passerebbero comunque.
  Dalle porte invece passa ogni modifica, tranne `undo` e `redo`
  #rapporto("BUF-1"). Il punto si muove liberamente: leggere è lo scopo di un
  buffer in sola lettura.
+ *I dati appesi vivono quanto il buffer.* Ciò che una funzionalità ricorda
  #emph[di un buffer] --- i risultati di una ricerca mostrati lì, una tabella,
  lo stato di una vista --- sta in `data`, sotto un nome, come
  `Box<dyn Any>`. Uccidere il buffer distrugge la mappa, e ogni valore con il
  suo `Drop`; un `downcast` del tipo sbagliato risponde `None`. Due
  funzionalità appendono i loro dati allo stesso buffer senza sapere l'una
  dell'altra. Scartati: una tabella globale per nome di buffer, sbagliata dopo
  un cambio di nome e ancora lì dopo la chiusura; il testo del buffer, che va
  bene solo per ciò che si mostrerebbe comunque; un puntatore grezzo, che non
  ha distruttore e che con un cast sbagliato è un comportamento indefinito
  invece di un rifiuto.
+ *`adopt_text` sostituisce il testo in un metodo solo.* Sei istruzioni
  sparse nei chiamanti sono sei occasioni di dimenticarne una, e quelle facili
  da dimenticare non si vedono subito: senza la versione l'evidenziatore
  salverebbe i colori del testo vecchio sul testo nuovo, senza svuotare gli
  overlay una diagnostica resterebbe su una riga che si è spostata. Butta via
  undo, mark, overlay e testo virtuale, che descrivono posizioni di un testo
  che non c'è più: disfare verso uno stato che il file non ha mai avuto è
  peggio che non poter disfare. Tiene il modo, il limite dell'undo, e la riga
  e la colonna del punto, perché tornare circa dove si era è lo scopo di
  ricaricare un file che si sta leggendo.
+ *Il flag del rettangolo sta sul mark.* È una proprietà della selezione, non
  del buffer: ogni modifica disattiva il mark, e così chiude anche il modo
  rettangolo, senza una seconda cosa da ricordare di spegnere. Cambia solo ciò
  che si disegna: i comandi dei rettangoli lavorano da punto e mark in ogni
  caso.
+ *Gli overlay seguono le modifiche, alle porte.* Le regole di un modo sono
  espressioni regolari sul testo: dicono «una parola di questa lista è una
  parola chiave», non «#emph[questo] tratto, per qualcosa che è successo». Per
  dire la seconda cosa ci sono gli overlay, che quindi devono seguire il
  testo: le porte chiamano `adjust_for_insert` e `adjust_for_delete`, come per
  l'undo e la versione. Una posizione che si sposta senza i suoi overlay è
  un'evidenziazione sulle parole sbagliate, minuti dopo, senza niente che dica
  perché.
+ *La tabella degli overlay è ordinata per inizio.* Una modifica sposta gli
  overlay che stanno dopo di lei: con l'ordine sono una ricerca binaria e una
  coda, invece di tutta la tabella. Le due regole di aggiustamento sono
  monotone, quindi l'ordine sopravvive e non si ricostruisce mai. Il caso più
  comune, una modifica dopo l'ultimo overlay, costa un confronto con
  `furthest_end`.
+ *Nessuna estremità di un overlay cresce.* `start` si sposta quando
  l'inserimento è prima o proprio lì, `end` solo quando è strettamente prima:
  un testo inserito all'inizio o alla fine resta fuori, uno inserito dentro
  entra a farne parte. Una corrispondenza evidenziata resta esattamente il
  testo che corrispondeva, qualunque cosa si scriva accanto.
+ *Il testo virtuale è un punto, in una tabella sua.* Non è un overlay: un
  overlay colora caratteri che ci sono, e uno senza larghezza viene
  cancellato. Non è nemmeno testo inserito davvero: il buffer risulterebbe
  modificato, l'undo si riempirebbe di cose mai scritte e il salvataggio le
  scriverebbe nel file. È una posizione che si sposta con il testo e non
  sparisce mai. La tabella non sa a che cosa serve: suggerimenti, anteprime e
  il resto sono categorie scelte da chi li produce.
+ *Due cache, perché due lessici.* La grammatica decide che aspetto ha il
  testo, e il suo stato è la pila delle regioni aperte; la tabella sintattica
  decide che cosa significa --- dove sono stringhe, commenti e liste --- e il
  suo stato è un `Resume`. `forward-sexp`, l'indentazione, `syntax-ppss` ed
  electric-pair leggono la seconda, e nessuno di loro si può servire dalla
  prima (@modi).
+ *La colorazione tiene uno stato per riga, la scansione un punto ogni 64
  righe.* Quasi ogni riga di quasi ogni file comincia nello stesso stato
  vuoto, e gli stati della colorazione, internati, costano poco. Uno stato
  della scansione contiene invece le #emph[posizioni] delle liste aperte, così
  due righe alla stessa profondità hanno stati diversi e internarli non
  risparmia nulla. Un punto di ripresa ogni 64 righe costa un sessantaquattresimo
  della memoria, e una risposta al più 64 righe di scansione.
+ *Le cache stanno sul buffer.* Ciò che le invalida è una modifica, e le
  modifiche passano dalle porte; ciò che descrivono è un testo, a cui non
  devono sopravvivere. Una tabella a parte nell'editor andrebbe ripulita a
  ogni chiusura di un buffer, e sarebbe sbagliata, invece che assente, il
  giorno in cui non lo fosse.
+ *Chi legge la cache della scansione non la scrive.* La riempie solo lo
  scanner preventivo, e i movimenti la leggono con il lock di lettura che
  hanno già. Riempirla leggendo vorrebbe dire prendere il lock di scrittura del
  buffer sul percorso di un tasto, nel mezzo della valutazione di Lisp, per
  far funzionare una cache. Il prezzo: un file appena aperto si scandisce
  dall'inizio finché lo scanner preventivo non l'ha percorso.
+ *Una modifica tiene ciò che sta sopra.* Una modifica alla riga L non cambia
  il testo prima di L: i punti di ripresa e gli stati che entrano nelle righe
  fino a L restano esatti, e si scarta solo da L in giù, senza dover contare
  le righe aggiunte o tolte.
+ *Il colore vecchio resta disegnato.* Dopo una modifica nulla di ciò che sta
  sotto è più affidabile, ma gli intervalli restano e si disegnano finché
  l'evidenziatore non li sostituisce. Cancellarli farebbe passare il resto del
  file al testo semplice a ogni tasto, che è molto peggio di qualche
  millisecondo di colore un po' sbagliato.
+ *La versione non basta.* Un cambio di modo, o una regola aggiunta alla
  grammatica di un modo, non tocca il testo. Per questo le due cache ricordano
  anche il modo e l'edizione della grammatica o della tabella con cui sono
  state calcolate, e una cache che ne descrive un'altra ricomincia
  dall'inizio. Così resta fuori dal lessico anche lo stato di un'altra
  grammatica, che nomina le regioni con il loro posto in quella.
+ *La colorazione non si risincronizza.* Dopo una modifica si ricalcola fino
  alla fine del file, invece di fermarsi dove lo stato calcolato torna uguale
  a quello di prima. Fermarsi lì costerebbe le righe fino alla convergenza
  invece del resto del file, ma richiede di tenere gli stati della
  generazione precedente e di contare di quante righe la modifica ha spostato
  il resto: senza quel conto si confronterebbero le righe sbagliate. Il lavoro
  a passi limitati tiene comunque l'editor reattivo, e la risincronizzazione
  riguarda quanto lavoro c'è in tutto: si aggiunge se le misure la chiedono.
+ *Undo scrive sotto le porte.* Rifare la storia non deve registrarsi come
  storia nuova, quindi `undo` e `redo` applicano le modifiche con
  `apply_insert` e `apply_delete`, sotto lo strato che registra: non c'è un
  flag da ricordare. Il chiamante fa poi ciò che le porte farebbero, con
  `changed` da `Applied::from` #rapporto("BUF-1").
+ *Il testo da rifare si cattura disfacendo.* Disfare un inserimento toglie il
  testo e lo ricorda nel gruppo inverso, invece di conservarlo prima per un
  `redo` che può non venire mai.
+ *Una macro è un gruppo solo.* Rieseguire una macro passa ogni comando per la
  via normale, e lascerebbe cinquanta gruppi dietro una macro di cinquanta
  comandi: chi la esegue non vuole disfarne un cinquantesimo. `OneGroup` tiene
  fermo il timbro per tutta la riproduzione, e la regola che c'è già --- un
  gruppo si chiude alla prima modifica sotto un altro timbro --- fa il resto.
+ *Il file si riconosce da un timbro, non da un hash.* La domanda «il file è
  cambiato?» si fa per ogni buffer aperto, ogni pochi secondi. Un hash
  risponde esattamente ma legge tutto il file; un timbro --- istante di
  modifica e lunghezza --- risponde con un solo `stat`. Sfugge una scrittura
  che cada nella granularità del filesystem e lasci identica la lunghezza: il
  buffer resta vecchio fino al cambiamento successivo, senza danni.
+ *Il timbro si confronta per differenza, non per «più recente».* Gli istanti
  tornano indietro nell'uso normale: `git checkout` rimette istanti vecchi,
  `touch -d` scrive quello che gli si dice, un filesystem di rete non è
  d'accordo con l'orologio locale. «Diverso da ciò che ho registrato» è una
  domanda a cui il filesystem risponde; «più recente del mio» è una domanda su
  due orologi. Il timbro si registra anche a ogni scrittura dell'editor,
  altrimenti ogni salvataggio sembrerebbe la scrittura di qualcun altro.
+ *`stale` è un flag, non una domanda.* Il momento in cui fermare la
  sovrascrittura del lavoro di un altro è il salvataggio, non l'istante in cui
  il cambiamento si nota: il watcher marca il buffer, e il salvataggio
  successivo chiede.
+ *L'autosalvataggio confronta versioni.* «Modificato» resta vero dal primo
  tasto fino al salvataggio, quindi un lavoro che scrivesse a ogni turno ogni
  buffer modificato riscriverebbe per sempre un buffer fermo. La versione si
  muove solo quando si muove il testo: `auto_saved_at` la ricorda, e la
  domanda «c'è qualcosa da scrivere?» diventa un confronto tra due numeri. Un
  editor lasciato aperto di notte non costa nulla.

// ===========================================================================
= Il testo: ricerca, sostituzione, kill ring, rettangoli e liste di risultati <testo>
// ===========================================================================

La directory `core/src/text/` raccoglie ciò che si può calcolare su un testo
senza sapere che cosa sia un editor. Il criterio di ammissione è scritto in
`text/mod.rs`: un file sta qui se lo potrebbe usare un programma che non è un
editor. In pratica nessun file della directory importa `editor` o `lisp`, e
nessuno riceve un `Buffer`: ricevono un `BufferTrait` (o una `&str`), cioè
qualcosa che sa dare caratteri per posizione, e nient'altro. Niente punto,
mark, undo, lock o finestre.

#ref-table(
  columns: (3.4cm, 1fr, 4.2cm),
  header: ([File], [Che cosa contiene], [Chi lo guida]),
  [`search.rs`], [`Pattern` (che cosa cercare) con `search_forward`, `search_backward` e `scan`; le espressioni compilate di `string-match` (`compiled`).], [`feature/isearch.rs`, `editor/replace.rs`, `primitives/scan.rs`, `primitives/match_data.rs`],
  [`isearch.rs`], [La sessione di ricerca incrementale: `Isearch` e `Direction`.], [`feature/isearch.rs`, `primitives/isearch.rs`],
  [`replace.rs`], [La sessione di sostituzione, `Replace`, con `Casing` ed `expand_replacement`.], [`editor/replace.rs`],
  [`kill_ring.rs`], [`KillRing`: un anello di stringhe con la regola di rotazione e di accodamento.], [`managers/kill_yank.rs` (`KillYank`)],
  [`rectangle.rs`], [`Rectangle` e `Span`: l'aritmetica di due angoli e delle righe tra loro.], [`primitives/rectangle.rs`, `ui/windows.rs`],
  [`results.rs`], [`Entry` e `Results`: una lista di posti e un cursore che la percorre.], [`primitives/results.rs`, `editor/results.rs`],
)

Le sessioni (`Isearch`, `Replace`) tengono lo #emph[stato] di una ricerca o di
una sostituzione tra un tasto e l'altro. Chi le guida le conserva in `Runtime`
e chiama un metodo per ogni risposta dell'utente; la sessione conosce il
buffer solo per nome (@decisioni-testo).

== Posizioni: sempre caratteri

Tutte le posizioni del modulo sono offset in #emph[caratteri], come il punto, il
mark e la storia dell'undo. Il motore delle espressioni regolari (`regex`)
lavora invece in #emph[byte] su una `&str`. La conversione avviene in un posto
solo: `char_to_byte` all'ingresso e `to_match` (che usa `byte_to_char`)
all'uscita. Un errore di conversione non si vede su testo ASCII e rompe il
primo carattere accentato, per questo non è ripetuta nei chiamanti.

#canvas(height: 3.0cm)[
  #node(2.6cm, 0.5cm, 4.2cm, 0.6cm, [`"città sì"`: 9 caratteri], fill: paper)
  #node(2.6cm, 2.4cm, 4.2cm, 0.6cm, [la stessa stringa: 11 byte], fill: paper)
  #note(1.4cm, 1.3cm)[`à` e `ì` occupano 2 byte ciascuna]
  #node(11.0cm, 0.5cm, 5.6cm, 0.6cm, [caratteri: punto, mark, `Match`, `Found`])
  #node(11.0cm, 2.4cm, 5.6cm, 0.6cm, [byte: `regex::captures_at`])
  #arrow((9.6cm, 0.85cm), (9.6cm, 2.05cm), label: [`char_to_byte`], label-dx: -62pt, label-dy: -4pt)
  #arrow((12.4cm, 2.05cm), (12.4cm, 0.85cm), label: [`to_match`], label-dx: 6pt, label-dy: -4pt)
]

== `Pattern`: che cosa cercare <ricerca>

#estratto("text/search.rs")[
```rust
pub enum Pattern {
    Literal { chars: Vec<char>, fold: bool },   // confronto carattere per carattere
    Regex(Regex),                               // compilata una volta sola
}
pub fn new(source: &str, regexp: bool, fold: bool) -> Result<Self, String> {
    if source.is_empty() {
        return Err("Searching for the empty string would never finish".into());
    }
    if regexp {
        RegexBuilder::new(source).case_insensitive(fold).multi_line(true)
            .build().map(Pattern::Regex).map_err(|why| format!("Invalid regexp: {why}"))
    } else {
        Ok(Pattern::Literal { chars: source.chars().collect(), fold })
    }
}
```
]

Tre decisioni stanno in questa funzione:

- #emph[Il modello vuoto è rifiutato.] Troverebbe una corrispondenza in ogni
  posizione: una ricerca non si muoverebbe mai e una sostituzione non
  finirebbe. Rifiutandolo qui nessun chiamante deve ricordarsi il controllo.
  Un'espressione che #emph[può] corrispondere al vuoto (`x*`) resta invece
  ammessa, ed è compito di chi cicla scavalcare le corrispondenze vuote
  (`Match::is_empty`) #rapporto("RIC-1").
- #emph[`multi_line(true)`]: `^` e `$` indicano inizio e fine di #emph[riga],
  come si aspetta chi scrive un'espressione in un editor. Senza, `^;;`
  troverebbe al massimo una corrispondenza in tutto il buffer.
- #emph[`fold`] arriva dal chiamante, che lo legge dalla variabile Lisp
  `case-fold-search` (non legata = vero). Per i letterali il confronto
  ignora le maiuscole con `chars_equal`, che confronta le forme minuscole dei
  due caratteri.

=== `search_forward` e `search_backward`

`search_forward(text, from, limit)` restituisce la prima corrispondenza che
#emph[comincia] in `from` o dopo e #emph[finisce] entro `limit`. Il limite è
ciò che confina una sostituzione alla regione: il chiamante passa la fine
della regione invece di quella del buffer.

- #emph[Letterale]: prova ogni inizio da `from` a `limit - len` con
  `matches_at`, che legge i caratteri uno per uno con `BufferTrait::at`. Non
  alloca nulla: un passaggio, nessuna copia.
- #emph[Espressione]: copia il buffer in una `String` (il motore vuole una
  `&str`), converte `from` in byte e chiama `captures_at`. Si usa
  `captures_at` e non una fetta del testo perché la ricerca deve vedere ciò
  che precede il punto di partenza: altrimenti ogni ripresa sembrerebbe
  l'inizio di una riga per `^` e per i look-behind. La corrispondenza trovata
  è scartata se finisce oltre il limite, anche quando ne esisterebbe una più
  corta che ci sta #rapporto("RIC-4").

#estratto("text/search.rs")[
```rust
Pattern::Regex(_) => {
    let mut best = None;
    let mut at = floor;
    while let Some(found) = self.search_forward(text, at, to) {
        at = if found.is_empty() { found.end + 1 } else { found.end };
        best = Some(found);
    }
    best
}
```
]

`search_backward(text, to, floor)` cerca l'ultima corrispondenza che finisce
entro `to` e comincia da `floor` in poi. Per i letterali prova gli inizi
all'indietro. Per le espressioni il motore sa cercare solo in avanti, quindi
cammina tutte le corrispondenze da `floor` e tiene l'ultima: ogni passo è una
`search_forward`, che ricopia il buffer, e il costo totale cresce con il
quadrato del testo #rapporto("RIC-2").

=== `scan`: tutte le corrispondenze in un passaggio

Occur, grep e la lettura dell'output di un compilatore vogliono #emph[tutte]
le corrispondenze, con riga, colonna e testo della riga. Chiamare
`search_forward` in un ciclo costerebbe una copia del buffer per corrispondenza.
`Pattern::scan(haystack, limit)` lavora su una `&str` già pronta e fa un
passaggio solo, grazie a un `Cursor` che avanza sempre in avanti e conta
caratteri e righe man mano:

#estratto("text/search.rs")[
```rust
struct Cursor<'a> {
    haystack: &'a str, byte: usize, chars: usize, line: usize, line_start: usize,
}
fn advance_to(&mut self, byte: usize) {          // byte >= self.byte, sempre
    for (at, c) in self.haystack[self.byte..byte].char_indices() {
        self.chars += 1;
        if c == '\n' { self.line += 1; self.line_start = self.byte + at + c.len_utf8(); }
    }
    self.byte = byte;
}
```
]

Per ogni corrispondenza (in byte, da `next_match`) `scan` porta il cursore al
suo inizio, legge riga e colonna, prende il testo della riga (memorizzato, così
venti corrispondenze sulla stessa riga non la ricostruiscono venti volte) e
converte inizio e fine in caratteri. Poi riprende dalla fine della
corrispondenza, o un carattere dopo se era vuota. Il risultato è una `Scan`:
il vettore dei `Found` e `truncated`, vero se si è fermata per `limit`. La
colonna invece è ricalcolata contando dall'inizio della riga a ogni
corrispondenza #rapporto("RIC-4").

#ref-table(
  columns: (2.6cm, 1fr),
  header: ([Campo di `Found`], [Significato]),
  [`start`, `end`], [Offset in caratteri nell'intero testo.],
  [`line`], [Riga di inizio, contata da 1 (la numerazione di `goto-line`).],
  [`column`], [Colonna in caratteri, contata da 0 (come `current-column`).],
  [`line_text`], [La riga intera senza `\n`: mostrarla non richiede di rileggere il testo.],
  [`groups`], [Ciò che ha catturato ogni gruppo, 0 = l'intera corrispondenza; vuoto per un letterale.],
)

`in_line()` ed `end_in_line()` danno la posizione della corrispondenza dentro
`line_text`, che è ciò che serve a una vista per evidenziarla.

=== Le espressioni di `string-match`

`string-match` e `string-match-p` non costruiscono un `Pattern`: chiedono a
`compiled(pattern)` l'espressione già compilata. Ogni thread tiene la sua
tabella, dalla sorgente all'espressione (`Arc<Regex>`), e la svuota tutta
quando arriva a `CACHE_LIMIT`, 512 voci.

== La sessione di ricerca incrementale: `Isearch` <isearch>

La parte di interfaccia è descritta nel @minibuffer: `C-s` apre il prompt in
`isearch-mode` e il `post-command-hook` di quel modo chiama `isearch-update`
dopo ogni comando. Qui interessa lo stato che passa da un tasto all'altro.

#estratto("text/isearch.rs")[
```rust
pub struct Isearch {
    pub buffer: String,          // il buffer cercato (il corrente è il minibuffer)
    pub origin: usize,           // dove era il punto: ripristinato da C-g
    pub direction: Direction,    // Forward | Backward
    pub regexp: bool,
    pub from: usize,             // da dove parte la prossima scansione
    pub found: Option<Match>,    // la corrispondenza mostrata
    pub failing: bool,           // l'ultima scansione non ha trovato nulla
    pub wrapped: bool,           // la ricerca ha fatto il giro
}
```
]

Il campo chiave è `from`. Scrivere un carattere #emph[non] lo sposta: allungare
il modello rifà la ricerca dallo stesso punto, quindi la corrispondenza cresce
dove l'utente sta guardando. Solo la ripetizione lo sposta.

#canvas(height: 5.6cm)[
  #node(2.2cm, 0.6cm, 3.2cm, 0.65cm, [carattere / `BS`])
  #node(2.2cm, 2.2cm, 3.2cm, 0.65cm, [`C-s` / `C-r`])
  #node(8.4cm, 2.2cm, 5.0cm, 0.65cm, [`repeat`: gira, `advance` o `wrap`], fill: paper)
  #node(8.4cm, 4.2cm, 6.4cm, 1.0cm, [`isearch-update`: modello da `*Minibuffer*`,\ `Pattern::new`, ricerca da `from`])
  #node(14.6cm, 4.2cm, 3.4cm, 1.0cm, [trovato: `show`\ altrimenti `failing`], fill: paper)
  #arrow((3.0cm, 0.93cm), (5.5cm, 3.65cm))
  #arrow((3.8cm, 2.2cm), (5.8cm, 2.2cm))
  #arrow((8.4cm, 2.55cm), (8.4cm, 3.65cm), label: [poi il hook], label-dx: 30pt, label-dy: -2pt)
  #arrow((11.65cm, 4.2cm), (12.85cm, 4.2cm))
  #note(10.6cm, 0.5cm)[ogni comando eseguito nel prompt (un carattere,\ un `BS`, un `C-s`) fa scattare `post-command-hook`,\ che chiama `isearch-update`]
]

Le regole dei metodi, tutte prese dal comportamento di Emacs:

- `advance()` sposta `from` a `found.start + 1` andando avanti (a
  `found.end - 1` andando indietro), non alla fine della corrispondenza: così
  anche le corrispondenze sovrapposte sono raggiungibili. Ripetere la ricerca
  di `aa` in `aaa` trova anche quella che comincia in 1.
- `wrap(len)` riparte dall'altro capo del buffer e imposta `wrapped`. Viene
  chiamato solo quando si ripete una ricerca che sta #emph[già] fallendo: il
  primo `C-s` a vuoto mostra «Failing I-search», il secondo è la decisione di
  fare il giro.
- Cambiare direzione non avanza: `from` diventa l'inizio (o la fine) della
  corrispondenza corrente, perché `C-s C-r` deve ripercorrere le stesse
  corrispondenze senza saltarne una.
- `report(pattern)` compone il testo per l'area dei messaggi:
  `[Failing ][Wrapped ]` seguito da `I-search`, `I-search backward`,
  `Regexp I-search` o `Regexp I-search backward`, poi `: ` e il modello.

`isearch-update` gestisce anche i casi limite. Con il prompt vuoto riporta il
punto a `origin` e toglie il mark. Un'espressione che non si compila (`[a-` è
a metà di `[a-z]`) è trattata come una ricerca fallita, non come un errore:
il carattere successivo la renderà valida. Quando trova, `show` mette il punto
sull'estremità lontana della corrispondenza e il mark su quella vicina, così la
corrispondenza è la regione e la disegna lo stesso codice che disegna una
selezione. `isearch-exit` (Invio) disattiva il mark e lascia il punto dov'è;
`isearch-abort` (`C-g`, `ESC`) riporta il punto a `origin` #rapporto("RIC-6").

== La sessione di sostituzione: `Replace` <sostituzione>

`replace-string`, `replace-regexp` e `query-replace` creano una `Replace` con
`begin_replace` (`editor/replace.rs`). Il campo d'azione è la regione se il mark
è attivo, altrimenti dal punto alla fine del buffer: `origin` e `limit` sono i
due estremi. Poi `seek` cerca la prima corrispondenza e un overlay della
categoria `replace-match` la evidenzia.

#estratto("text/replace.rs")[
```rust
pub struct Replace {
    pub buffer: String, pub pattern: Pattern,
    pub replacement: String,     // il modello, con \1 \& ancora dentro
    pub origin: usize, pub from: usize,
    pub limit: usize,            // spostato da ogni sostituzione
    pub found: Option<Match>, pub replaced: usize,
    pub history: Vec<Step>,      // da dove partiva ogni sostituzione accettata
    pub preserve_case: bool, pub done: bool,
}
```
]

Chi la guida chiama un verbo per ogni risposta. La vista predefinita
(`default_view` in `primitives/replace.rs`) installa una keymap transitoria con
cinque tasti: `y` (`replace-this`), `n` (`replace-skip`), `!` (`replace-rest`),
`^` (`replace-back`, che usa `unaccept` e l'undo del buffer) e `q`
(`replace-done`). La keymap è di tipo `OnUnbound::Release`: qualunque altro
tasto toglie la domanda e poi fa ciò che fa di solito, ma la sessione resta
aperta e la corrispondenza evidenziata #rapporto("RIC-8"). Un'interfaccia
grafica userebbe pulsanti; nessuna delle due si esprime come un ciclo che
legge una risposta.

#ref-table(
  columns: (3.0cm, 1fr),
  header: ([Verbo], [Effetto sulla sessione]),
  [`seek(text)`], [`found = search_forward(text, from, limit)`; se non trova nulla, `done = true`.],
  [`accept(new_len)`], [La corrispondenza è stata sostituita da `new_len` caratteri. Registra `Step { from: start, limit }` in `history`; sposta `limit` della differenza di lunghezza; riprende da `start + new_len` (o `start + 1` se corrispondenza e sostituzione erano entrambe vuote) #rapporto("RIC-1"). Incrementa `replaced`.],
  [`decline()`], [Salta la corrispondenza: riprende dalla sua fine, o un carattere dopo se era vuota.],
  [`unaccept()`], [Riprende l'ultimo `Step`: `from` e `limit` tornano com'erano, `replaced` cala. Il testo lo rimette a posto il chiamante con l'undo del buffer, perché la sessione non sa nulla dei buffer.],
)

Il limite si sposta perché una sostituzione più lunga o più corta sposta tutto
ciò che segue, compresa la fine della regione. Senza questo aggiornamento una
sostituzione confinata alla regione ne uscirebbe, o si fermerebbe prima. La
`history` serve a tornare indietro: dopo una sostituzione di lunghezza diversa
il testo da solo non dice più dove era cominciata.

#canvas(height: 2.9cm)[
  #node(1.6cm, 0.6cm, 2.6cm, 0.65cm, [`begin_replace`])
  #node(5.6cm, 0.6cm, 2.6cm, 0.65cm, [`seek`], fill: paper)
  #node(10.0cm, 0.6cm, 3.4cm, 0.65cm, [match offerto])
  #node(14.2cm, 0.6cm, 2.2cm, 0.65cm, [`done`], fill: paper)
  #node(8.0cm, 2.2cm, 3.4cm, 0.65cm, [`accept` / `decline`])
  #arrow((2.9cm, 0.6cm), (4.3cm, 0.6cm))
  #arrow((6.9cm, 0.6cm), (8.3cm, 0.6cm), label: [trovato])
  #arrow((11.7cm, 0.6cm), (13.1cm, 0.6cm), label: [`q`, altro])
  #arrow((10.0cm, 0.93cm), (9.2cm, 1.87cm), label: [`y`, `n`, `^`], label-dx: 22pt, label-dy: 0pt)
  #arrow((6.8cm, 2.2cm), (5.6cm, 0.93cm), label: [`seek`], label-dx: -30pt, label-dy: 0pt)
  #arrow((5.6cm, 0.27cm), (14.2cm, 0.27cm), dash: "dashed")
  #note(7.2cm, -0.25cm)[non trovato]
]

Il testo da inserire lo calcola, per ogni corrispondenza,
`expand_replacement`, che riceve il modello di sostituzione, la corrispondenza,
il testo effettivamente trovato e `preserve_case`:

- `\1`…`\9` sono i gruppi catturati; un gruppo che non ha partecipato, o
  qualsiasi gruppo con un modello letterale (che non ne ha), diventa la
  stringa vuota;
- `\0` e `\&` sono l'intera corrispondenza, presa dal testo (`matched`) e non dal
  gruppo 0, così funzionano anche con i letterali;
- `\\` è una barra rovesciata; una barra seguita da qualunque altra cosa resta
  com'è, così un percorso Windows si può scrivere senza raddoppiare le barre.

Con `preserve_case` (il valore di `case-replace` quando la ricerca ignora le
maiuscole) il risultato viene poi riscritto secondo `Casing::of(matched)`:

#ref-table(
  columns: (3.2cm, 1fr, 3.6cm),
  header: ([Corrispondenza], [`Casing`], [`foo` diventa]),
  [`BAR`], [`Upper`: tutte le lettere maiuscole (almeno due)], [`FOO`],
  [`Bar`, `B`], [`Capitalised`: la prima maiuscola, le altre minuscole], [`Foo`],
  [`bar`, `bAr`, `fooBar`, `42`], [`AsWritten`: minuscole, miste o nessuna lettera], [`foo`],
)

Sul lato dell'editor, `replace_this` cancella la corrispondenza e inserisce
l'espansione con le due porte (`delete_range`, `insert_text`), chiude un gruppo
di undo #rapporto("RIC-3"), chiama `accept` e poi cerca la successiva.
`replace_rest` (il `!` di `query-replace`, e tutto `replace-string`) ripete
`replace_this` con un tetto di `len + 1` giri, che è l'unica cosa che ferma
il caso delle corrispondenze vuote #rapporto("RIC-1").

== Il kill ring <kill-ring>

#estratto("text/kill_ring.rs")[
```rust
pub struct KillRing {
    entries: VecDeque<String>,  // la più recente in testa
    max: usize,                 // DEFAULT_KILL_RING_MAX = 60
    index: usize,               // da dove legge yank; azzerato da ogni kill
}
pub fn append(&mut self, text: String, direction: Direction) {
    if text.is_empty() { return; }
    match self.entries.front_mut() {
        Some(front) => {
            match direction {
                Direction::Forward => front.push_str(&text),
                Direction::Backward => front.insert_str(0, &text),
            }
            self.index = 0;
        }
        None => self.push(text),
    }
}
```
]

Un anello invece di un appunto singolo: uccidere del testo non distrugge quello
che si stava per incollare. Le operazioni:

- `push(text)` aggiunge una voce in testa e azzera `index`; il testo vuoto
  è ignorato, così una kill di niente non allontana ciò che si voleva incollare;
  poi `trim` scarta le voci oltre `max`.
- `append(text, direction)` unisce il testo alla voce più recente: in coda
  per una kill in avanti, in testa per una all'indietro. Una serie di `C-k`
  diventa così un'unica voce che si legge nell'ordine giusto, e una serie di
  `M-DEL` non viene assemblata al contrario.
- `current()` è ciò che inserisce `yank`; `rotate()` avanza `index` di uno (con
  giro) ed è ciò che usa `yank-pop`; `nth(n)` legge senza muovere l'anello.
- `set_max` (`set-kill-ring-max`) riduce subito l'anello; se `index` finisce
  oltre la fine torna a 0.

La decisione tra `push` e `append` non è dell'anello ma del compartimento
`KillYank` (`managers/kill_yank.rs`), che tiene due coppie di flag:
`killed_last`/`killed_this` e `yanked_last`/`yanked_this`. Una kill accoda se
anche il comando precedente era una kill; `roll_over`, chiamato all'inizio di
ogni comando, sposta «questo» in «precedente» e azzera «questo». Allo stesso
modo `yank-pop` è permesso solo se il comando precedente era uno yank, e
`last_yank` dice quale tratto di testo sostituire.

#canvas(height: 2.3cm)[
  #node(1.8cm, 0.6cm, 2.4cm, 0.6cm, [`C-k` (kill)])
  #node(5.4cm, 0.6cm, 2.4cm, 0.6cm, [`C-k` (kill)])
  #node(9.0cm, 0.6cm, 2.4cm, 0.6cm, [`C-f`])
  #node(12.6cm, 0.6cm, 2.4cm, 0.6cm, [`C-k` (kill)])
  #note(0.9cm, 1.15cm)[`push`]
  #note(4.3cm, 1.15cm)[`append` (prec. = kill)]
  #note(8.2cm, 1.15cm)[`killed_this` resta falso]
  #note(11.7cm, 1.15cm)[`push`: nuova voce]
  #arrow((3.0cm, 0.6cm), (4.2cm, 0.6cm))
  #arrow((6.6cm, 0.6cm), (7.8cm, 0.6cm))
  #arrow((10.2cm, 0.6cm), (11.4cm, 0.6cm))
  #note(0.2cm, 1.75cm)[tra un comando e l'altro `roll_over`: `killed_last = killed_this; killed_this = false`]
]

Se la variabile `clipboard-sync` è vera (la imposta `clipboard.lisp`), ogni
kill lascia anche la voce corrente in `pending_clipboard`. La `snapshot`
successiva la prende (`take_pending_clipboard`) e il TUI la manda al terminale
con una sequenza OSC 52. Prenderla invece di leggerla evita di rimandarla a
ogni ridisegno.

L'ultimo rettangolo ucciso o copiato non va nell'anello: sta in un campo a
parte di `KillYank`, una riga per stringa, e `yank-rectangle` lo legge da lì.

== I rettangoli <rettangoli>

Il punto e il mark sono due angoli; il rettangolo è il blocco di colonne che le
righe tra loro hanno in comune. Non è la regione, che dalla metà di una riga
alla metà di un'altra prende anche le righe intere in mezzo.

#estratto("text/rectangle.rs")[
```rust
pub struct Rectangle { pub top: usize, pub bottom: usize, pub left: usize, pub right: usize }
pub fn between(a: (usize, usize), b: (usize, usize)) -> Self {   // (riga, colonna)
    Self { top: a.0.min(b.0), bottom: a.0.max(b.0), left: a.1.min(b.1), right: a.1.max(b.1) }
}
```
]

`bottom` è incluso e `right` no: le righe sono un insieme di #emph[cose], le
colonne un intervallo di #emph[posizioni]. Un rettangolo largo zero è ammesso
ed è utile: è una posizione su ogni riga, che è ciò che serve a
`string-rectangle` per anteporre un prefisso a un gruppo di righe. Gli angoli
possono essere dati in qualsiasi ordine.

Le colonne sono caratteri, e un tab è una colonna, come in tutto l'editor.
Sullo schermo il terminale lo espande, quindi su una riga con un tab il
blocco disegnato non è quello che il rettangolo prende #rapporto("TUI-3").

`spans(text, rect)` traduce il rettangolo in un `Span` per riga, dall'alto:

#estratto("text/rectangle.rs")[
```rust
(rect.top..=rect.bottom.min(last_line)).map(|line| {
    let line_start = text.cursor_2d_to_1d(line, 0);
    let start = text.cursor_2d_to_1d(line, rect.left);    // limitato alla fine della riga
    let end = text.cursor_2d_to_1d(line, rect.right);
    let reached = start - line_start;
    Span { line, start, end, padding: rect.left - reached.min(rect.left) }
})
```
]

Tutto si regge sul fatto che `cursor_2d_to_1d` limita una colonna oltre la
fine della riga alla fine della riga. Su una riga corta entrambi gli estremi
cadono sulla fine e lo span è vuoto; `padding` dice quanti spazi mancano per
raggiungere il bordo sinistro. Esempio con `left = 4`, `right = 7`:

#ref-table(
  columns: (3.2cm, 2.4cm, 2.4cm, 1fr),
  header: ([Riga], [Span], [`padding`], [Testo coperto]),
  [`0123456789`], [4..7], [0], [`456`],
  [`01234`], [4..5], [0], [`4` (la riga finisce prima del bordo destro)],
  [`01`], [2..2], [2], [vuoto],
  [(riga vuota)], [0..0], [4], [vuoto],
)

Che cosa fare delle righe corte lo decide l'operazione. Togliere testo
(`kill-rectangle`, `copy-rectangle-as-kill`, `clear-rectangle`) prende ciò che
c'è e non allunga la riga. Mettere testo (`open-rectangle`, `yank-rectangle`,
`string-rectangle`) prima aggiunge il padding, perché una colonna che non
esiste è proprio ciò che quelle operazioni devono riempire. Gli span sono
dall'alto perché è l'ordine di lettura; chi modifica li percorre dal basso,
perché una modifica a una riga sposta tutti gli offset successivi.
`text_of` restituisce il testo di ogni span, con una stringa vuota per le righe
che non raggiungono il rettangolo, così il blocco mantiene la sua forma.

== Le liste di risultati <risultati>

Occur, grep e la lettura dell'output di un compilatore producono la stessa
cosa: una lista di posti dove andare. `results.rs` la definisce una volta, così
produrre la lista e percorrerla sono lavori separati.

#ref-table(
  columns: (2.6cm, 1fr),
  header: ([Campo di `Entry`], [Significato]),
  [`kind`], [`"buffer"` o `"file"`: altrimenti `*scratch*` e un percorso si distinguerebbero solo indovinando.],
  [`source`], [Nome del buffer o percorso del file.],
  [`line`, `column`], [Riga da 1, colonna da 0.],
  [`offset`], [Offset in caratteri, per `goto-char`.],
  [`text`, `start`, `end`], [La riga intera e la posizione della corrispondenza al suo interno.],
  [`at`], [La riga dell'elenco su cui la voce è mostrata, da 1, quando il produttore la dice. La lista di una ricerca ha un'intestazione e poi una voce per riga, e non serve; l'uscita di un compilatore nomina i posti sulle righe dove il compilatore li ha scritti, e serve.],
)

`Entry::new(kind, source, found)` copia i campi da un `Found` di `scan`;
`rendered()` produce la forma `file:riga: testo` di ogni strumento alla grep.
`Results` aggiunge il modello cercato, su che cosa (`over`), le voci,
`truncated`, `done` (falso mentre una scansione di sfondo è in corso), il
buffer dove è stato messo l'ultimo evidenziatore (`marked`, per toglierlo) e
la voce corrente.

#estratto("text/results.rs")[
```rust
pub fn step(&self, step: isize) -> Option<usize> {
    if self.entries.is_empty() { return None; }
    let next = match self.current {
        None if step >= 0 => 0,                    // prima visita in avanti: la prima
        None => self.entries.len() - 1,            // prima visita indietro: l'ultima
        Some(current) => {
            let next = current as isize + step;
            if next < 0 || next as usize >= self.entries.len() { return None; }
            next as usize
        }
    };
    Some(next)
}
```
]

Il passo non fa il giro, di proposito: la fine della lista è un'informazione
(«era l'ultimo»), e ricominciare in silenzio farebbe correggere due volte il
primo errore. Un insieme di risultati vive nel buffer che lo mostra, in
`Buffer::data` sotto la chiave `"results"` (`RESULTS_KEY`): quando il buffer
viene chiuso i risultati spariscono con lui, e rinominarlo non rompe nulla.
Le voci conservano numeri di riga e offset, non marcatori che seguono il testo
#rapporto("RIC-5").

Dove una lista si mostra lo decide `editor/results.rs`. Accanto a ogni lista,
nei dati del suo buffer, sta una `Presentation`: chi l'ha prodotta, da quale
finestra e da quale buffer è stata chiesta, quale buffer ha preso il posto e
in quale finestra si vede. `prepare_results_buffer` sceglie il buffer di
un'esecuzione nuova: con `results-reuse-buffer`, la lista più recente dello
stesso produttore e dello stesso modo, fermando il lavoro che vi scriveva;
altrimenti un buffer nuovo. `display_results` la mostra nella finestra con il
focus, se `results-display-location` è `same-window`, o in una striscia in
basso, riusando quella che mostra una lista dello stesso produttore
#rapporto("RIC-7"). `select_results_target` sceglie la finestra in cui una
visita apre il posto (la lista stessa con `results-visit-same-window`), e
`quit_results` chiude la striscia o rimette il buffer che la lista aveva
sostituito.

== Le decisioni <decisioni-testo>

+ *`text/` è una directory, con una regola d'ammissione.* Un file sta qui se
  lo potrebbe usare un programma che non è un editor. Messi alla radice del
  crate, accanto a `editor/` e `ui/`, questi file farebbero una lista di
  moduli senza un principio, e lo strato sarebbe soltanto descritto. Una
  directory lo rende visibile e controllabile: `layering_tests` fallisce se un
  file di qui nomina l'editor o l'interprete.
+ *Ricerca e sessioni stanno sotto l'editor.* La ricerca ha bisogno solo di un
  testo che dia i caratteri per posizione: né punto, né mark, né undo, né
  lock. Così si prova su un buffer qualunque, senza un editor intorno, e la
  ricerca incrementale, la sostituzione e occur non possono avere due idee
  diverse di che cosa sia una corrispondenza. Lo stesso vale per l'aritmetica
  di una sostituzione: da dove riparte, di quanto sposta ciò che segue,
  quando si ferma.
+ *Un'espressione cerca su una copia del testo.* Il motore (`regex`) lavora su
  una `&str`, quindi la ricerca con un'espressione copia il buffer a ogni
  chiamata, mentre quella letterale non copia nulla. La ricerca incrementale
  ricerca a ogni tasto, e su un buffer grande una ricerca con un'espressione
  costa una copia per tasto. È la versione semplice; quella che scala darebbe
  il testo a pezzi, senza copiarlo, alla ricerca a flusso di
  `regex_automata`.
+ *Le espressioni di `string-match` si compilano una volta.* Compilarne una
  costa circa sette decimi di millisecondo: niente quando un modo dichiara la
  sua grammatica, un blocco quando succede in un ciclo. I moduli chiamano
  `string-match` per ogni elemento: la sorgente dei completamenti del manuale
  toglie l'estensione a ogni pagina che trova, e senza la cache quattromila
  pagine sarebbero quattromila compilazioni, 2,7 secondi a ogni Tab sul thread
  che disegna. Con la cache le stesse chiamate costano venti millisecondi.
+ *La cache è per thread, e quando è piena si svuota tutta.* Per thread,
  perché trovare una corrispondenza non prenda un lock: Lisp gira sul thread
  dei comandi e su quello dell'esecutore, e una tabella condivisa li
  metterebbe l'uno sulla strada dell'altro nella chiamata più frequente che
  c'è. Svuotarla tutta invece di scartare le voci vecchie basta: le
  espressioni vengono dal sorgente dei moduli e sono qualche decina, e il
  limite serve solo a un programma che le #emph[genera] dal suo input. Buttare
  tutto costa una compilazione per ognuna ancora in uso, cioè, nel caso
  peggiore, il comportamento senza la cache.
+ *Ricerca incrementale e sostituzione sono sessioni, non cicli.* Una ricerca
  incrementale è un ciclo sui tasti, e una sostituzione un ciclo sulle
  risposte; ma in rsedit non c'è una lettura bloccante di un tasto, perché i
  tasti arrivano uno per giro del ciclo degli eventi, e un pulsante di
  un'interfaccia grafica non è una lettura affatto. Il ciclo è quindi rovesciato,
  come la raccolta degli argomenti con `PendingCommand` (@comandi): la sessione
  tiene ciò che il ciclo terrebbe nelle sue variabili, e chi la guida chiama un
  metodo per ogni giro. La sessione non nomina buffer, finestre o lock, solo il
  nome del buffer e posizioni nel suo testo: si legge e si scrive senza tenere
  nulla, e descrive una ricerca in un buffer che non è il corrente, come è
  sempre mentre il prompt è aperto.
+ *La sostituzione segue le maiuscole di ciò che sostituisce.* Una ricerca che
  ignora le maiuscole e una sostituzione letterale trovano il posto giusto e
  ci scrivono la cosa sbagliata: sostituire `colour` con `color` in un
  documento trasforma il `Colour` a inizio frase in `color`, e ognuno va
  corretto a mano. È ciò che più si nota quando manca.
+ *Un anello solo per tutto l'editor.* Uccidere in un buffer e incollare in un
  altro è lo scopo del kill ring: l'anello sta sull'editor, in `KillYank`, e
  non sul buffer, così il testo passa da uno all'altro.
+ *Una riga corta resta nel rettangolo.* Una riga che finisce prima del bordo
  sinistro è comunque una delle righe del rettangolo, con uno span vuoto e il
  suo `padding`. Toglierla farebbe dipendere l'altezza del blocco da ciò che
  contiene.
+ *Il punto e l'evidenziazione della sostituzione stanno nell'editor, la
  domanda no.* Spostare il punto sulla corrispondenza serve a ogni vista, e una
  vista che dovesse farlo da sé potrebbe sbagliarlo; segnare la corrispondenza
  pure, perché una corrispondenza che non si vede è inutile in ogni vista. Ciò
  che resta alla vista è chiedere: nulla in `editor/replace.rs` mette una
  domanda sullo schermo o legge un tasto. Una vista che disegna da sé toglie
  l'overlay `replace-match`.

// ===========================================================================
= Modi <modi>
// ===========================================================================

== Il modo maggiore

Un buffer ha un solo modo maggiore, nominato in `current_mode`. Il modo è un
`MajorMode` nel registro di `Modes`:

#ref-table(
  columns: (4.6cm, 1fr),
  header: ([Campo], [Contenuto]),
  [`keymaps`], [Le associazioni del modo, consultate prima di quelle globali.],
  [`grammar`], [Regole e regioni di colorazione (`add-syntax-rule`, `add-syntax-region`).],
  [`hooks`], [Hook per nome (`post-command-hook`, `post-self-insert-hook`, `after-close-hook`, …), aggiunti con `(add-hook MODO NOME FUNZIONE)`.],
  [`completion_functions`], [Le fonti di `completion-at-point` per questo modo.],
  [`syntax_table`], [Delimitatori, stringhe, commenti, quote dei caratteri: ciò che legge lo scanner delle espressioni.],
)

`Modes` contiene inoltre la keymap globale, i completamenti globali, la lista
`auto_modes` (espressione regolare sul nome del file → modo, da
`add-auto-mode`), la keymap transitoria, i tasti di ripetizione e gli hook
globali, che valgono per ogni modo e si concatenano a quelli del modo.

`make-mode` registra un `MajorMode` nuovo sotto il nome dato, anche se il nome
esiste già: keymap, grammatica e hook definiti prima per quel nome vengono
sostituiti #rapporto("SIN-4").

Le proprietà dei simboli completano il modo senza toccare la struttura Rust;
queste sono lette da `indent.lisp`:

```lisp
(put 'rust-mode 'tab-width 4)
(put 'risp-mode 'indent-function 'lisp-indent-line)
(put 'rust-mode 'indent-region-style 'shift)
```

== Due lessici sullo stesso testo

Il testo di un buffer viene letto in due modi diversi, per due domande diverse.

#canvas(height: 4.6cm)[
  #node(8.1cm, 0.5cm, 6.0cm, 0.8cm, [testo del buffer (versione N)], fill: paper)
  #carrow((6.5cm, 0.9cm), (4.0cm, 1.6cm))
  #carrow((9.7cm, 0.9cm), (12.2cm, 1.6cm))
  #node(4.0cm, 2.1cm, 7.2cm, 1.0cm, [#strong[grammatica] (`modes/syntax.rs`)\ «che aspetto ha?» --- regex, riga per riga])
  #node(12.2cm, 2.1cm, 7.2cm, 1.0cm, [#strong[tabella sintattica] (`modes/sexp.rs`)\ «che cosa è?» --- carattere per carattere])
  #node(4.0cm, 3.7cm, 7.2cm, 0.9cm, [`Highlighter` → `SyntaxCache` → facce sullo schermo], fill: accent.lighten(90%))
  #node(12.2cm, 3.7cm, 7.2cm, 0.9cm, [`Prescanner` → `ScanCache` → `forward-sexp`,\ indentazione, `syntax-ppss`, `electric-pair`], fill: accent.lighten(90%))
  #carrow((4.0cm, 2.6cm), (4.0cm, 3.25cm))
  #carrow((12.2cm, 2.6cm), (12.2cm, 3.25cm))
]

La #emph[grammatica] colora una riga alla volta portando uno stato da una riga
all'altra: lo stato è la pila delle regioni aperte (un commento a blocchi che
continua, una stringa su più righe). Colorare la riga 5000 richiede solo lo
stato all'inizio della riga 5000, che la cache conserva; dopo una modifica si
ricolora dalla riga modificata in giù. In ogni riga tutti i modelli attivi sono
provati insieme e #emph[vince la corrispondenza più a sinistra]; a parità di
posizione le regioni vengono prima delle regole. Così `// this /* is not` è un
commento di riga e non apre un commento a blocchi. Le espressioni regolari
sono quelle della crate `regex`, senza lookahead, lookbehind né
backreference: per questo una regola può colorare solo un gruppo
(`SyntaxRule::group`) e una regione può dichiarare una sequenza di escape.

La #emph[tabella sintattica] dice quali caratteri aprono e chiudono liste,
quali delimitano stringhe (`set-string-syntax`, anche multi-carattere come
`r#"`), quali commenti (`set-comment-syntax`) e quale carattere quota un
carattere (`set-char-quote`). Le due descrizioni sono indipendenti: aggiungere
una forma di stringa a un modo significa aggiungerla in entrambe.

== Lo scanner delle espressioni

`modes/sexp.rs` risponde alle domande sulla struttura del testo: dove finisce
l'espressione dopo il punto (`forward-sexp`), dove comincia quella prima
(`backward-sexp`), in quale lista, stringa o commento si trova una posizione
(`syntax-ppss`, che l'indentazione e `electric-pair` usano). Contare le
parentesi non basta: in `(message "close it with )")` tre parentesi su quattro
sono testo. Lo scanner quindi fa un'analisi lessicale con quattro stati, e
conta un delimitatore solo quando lo sta leggendo come codice. Gli stati e le
transizioni:

#ref-table(
  columns: (3.6cm, 1fr, 1fr),
  header: ([Stato], [Ci si entra da `Code` quando…], [Si torna a `Code` quando…]),
  [`Code`], [--- (stato iniziale)], [---],
  [`Str { quote, start }`], [si legge un carattere di classe `StringQuote`.], [si rilegge la stessa virgoletta; un carattere `Escape` fa saltare anche il successivo.],
  [`StyledStr { style, start }`], [comincia un'apertura multi-carattere dichiarata con `set-string-syntax` (`r#"`, `"""`).], [si legge la chiusura dello stile. Nessun escape: in una stringa grezza `\` è solo `\`.],
  [`Comment { style, depth, start }`], [comincia un'apertura di commento di `set-comment-syntax` (vince la più lunga).], [riga: al `\n`. Blocco: alla chiusura con `depth` 0; se lo stile è `nestable` un'altra apertura incrementa `depth`.],
)

Lo stato completo di una scansione è lo stato lessicale più una pila di
`Frame`, uno per ogni lista aperta. Il fondo della pila è il buffer stesso e
non viene mai tolto: una chiusura in più non svuota mai la pila.

#estratto("modes/sexp.rs")[
```rust
struct Frame {
    start: usize,              // inizio dell'espressione, prefisso incluso
    delimiter: usize,          // dove sta la parentesi
    children: VecDeque<Sexp>,  // le ultime `keep` espressioni completate qui
}
pub struct Scan<'a, B: BufferTrait> {
    text: &'a B, table: &'a SyntaxTable,
    pos: usize, state: State, stack: Vec<Frame>,
    prefix: Option<usize>,     // un carattere prefisso (' ` # ,) in attesa
    keep: usize,
}
```
]

`step()` fa un passo e dice se ha completato un'espressione
(`Completed(depth, Sexp)`), se ha solo consumato caratteri (`Advanced`) o se il
testo è finito (`End`). In stato `Code` l'ordine dei controlli conta:

+ Un'apertura di commento (la più lunga che corrisponde) entra in `Comment` e
  scarta un prefisso in attesa. Anche un commento completato conta come
  espressione, così `forward-sexp` lo scavalca.
+ Un'apertura di stringa multi-carattere viene prima dei caratteri singoli,
  perché contiene virgolette che altrimenti aprirebbero una stringa normale.
+ Un letterale di carattere (`'a'`, `'"'`) è un'unica espressione. Il `'` non ha
  una classe fissa: apre un carattere in `'a'`, una lifetime in `'static`, un
  apostrofo nella prosa; lo decide `SyntaxTable::char_literal_at` guardando ciò
  che segue.
+ Poi la classe del carattere: `Open` mette un `Frame` sulla pila; `Close` toglie
  quello in cima e completa la lista. Una chiusura che non corrisponde
  all'apertura chiude lo stesso, e una chiusura senza niente di aperto è testo:
  un file è sbilanciato per tutto il tempo in cui lo si scrive, e uno scanner
  che si fermasse lì sarebbe inutile proprio quando serve. `StringQuote` entra in
  `Str`; `Escape` salta due caratteri; `Prefix` ricorda la posizione del primo
  prefisso di una serie (`#'foo` comincia al `#`); `Symbol` consuma l'intera
  serie di costituenti e completa un'espressione; `Punctuation` avanza, e uno
  spazio stacca un prefisso in attesa.

Ogni espressione completata entra nei `children` del frame in cima, che ne
tiene al massimo `keep`. È il limite che rende la scansione economica:
`backward-sexp` con un conteggio N ha bisogno solo delle ultime N.

#ref-table(
  columns: (4.0cm, 1fr),
  header: ([Funzione], [Come usa la scansione]),
  [`forward(from, n)`], [Scansione fino a `from`, poi passi finché non si completano `n` espressioni #emph[allo stesso livello] che finiscono dopo `from`. Se la lista finisce prima, il punto non si muove: alla fine di una lista il comando non esce verso l'esterno #rapporto("SIN-6").],
  [`backward(from, n)`], [Scansione fino a `from` con `keep = n`; il frame rimasto aperto è la lista in cui si trova il punto, e i suoi `children` sono i fratelli prima del punto. Restituisce l'inizio dell'N-esimo dal fondo, prefisso incluso.],
  [`down(from)`], [Scansione finché la profondità non supera quella di partenza: il punto è appena entrato in una lista.],
  [`context_at(pos)`], [Scansione fino a `pos`, poi `context()`: profondità, inizio e delimitatore della lista più interna, inizio della stringa o del commento in corso. È ciò che restituisce `syntax-ppss`.],
  [`balance_point(pos)`], [La prima posizione dopo `pos` in cui tutte le liste aperte in `pos` sono chiuse. Non chiede se il buffer è bilanciato alla fine: una funzione chiusa resta chiusa anche se quella sotto è ancora da finire.],
  [`enclosing(pos)`], [La lista più interna che contiene `pos`, per intero (fino alla fine del testo se non è chiusa); `None` al livello zero.],
)

=== Riprendere da metà: `Resume` e il `Prescanner`

Andare all'indietro non si può fare localmente: se una `)` prima del punto sia
codice o testo dipende da tutto ciò che la precede, perché una stringa aperta
duecento righe sopra la rende testo. Quindi ogni scansione va in avanti, e può
cominciare solo in una posizione di cui si conosce lo stato. All'inizio
quella posizione era solo la 0; ora `Scan::begin` accetta un `Resume`, una
fotografia dello stato presa prima:

#estratto("modes/sexp.rs")[
```rust
pub struct Resume { pos: usize, state: State, stack: Vec<Frame>, prefix: Option<usize> }
pub fn begin(text, table, before: usize, resume: Option<&Resume>, keep: usize) -> Self {
    if let Some(from) = resume.filter(|from| from.pos <= before) {
        return Self { pos: from.pos, state: from.state.clone(),
                      stack: from.stack.clone(), .. };          // riprende da lì
    }
    Self { pos: 0, state: State::Code,
           stack: vec![Frame { start: 0, delimiter: 0, .. }], .. }  // dall'inizio
}
```
]

Il controllo `from.pos <= before` è l'intero argomento di sicurezza: una
fotografia è solo una scorciatoia per arrivare a una posizione, mai un
sostituto. Una fotografia sbagliata o troppo avanti costa una scansione
completa, non una risposta sbagliata. Le fotografie sono prese senza i
`children` dei frame, che dipendono da `keep`: per questo `backward` non
riprende mai e scansiona sempre dall'inizio del buffer.

Le fotografie le produce il `Prescanner`, un lavoro di sfondo come
l'evidenziatore (@sfondo). Ne prende una ogni `LINES_PER_CHECKPOINT` (64)
righe, all'inizio delle righe 64, 128, 192… (`checkpoint_line`), e le salva
nella `ScanCache` del buffer insieme alla versione del testo e al nome del
modo. Una fotografia presa con la tabella di un altro modo sarebbe una risposta
sbagliata che sembra giusta, e per questo il modo è controllato. Dopo una
modifica alla riga L, `invalidate_from` tiene le fotografie delle righe fino a L
(lo stato all'ingresso di una riga dipende solo dal testo prima) e scarta le
altre; `record` accetta solo la fotografia successiva all'ultima valida.
`Buffer::scan_resume(pos)` sceglie quella della riga multipla di 64 più vicina
sotto `pos`, se versione e modo corrispondono.

== Facce e temi

Una faccia è un `Face(u16)`: un indice in un registro globale. Le prime 14 sono
predefinite (`default`, `region`, `mode-line`, `mode-line-inactive`,
`window-separator`, `keyword`, `type`, `string`, `comment`, `function`,
`builtin`, `line-number`, `line-number-current`, `replace-match`); un modulo
crea una faccia nuova semplicemente nominandola (`'doc-comment` in
`rust-mode.lisp`) e le dà un aspetto con `set-face`. Il `Theme` in `Runtime`
associa a ogni faccia uno `Style` (colori di primo piano e sfondo, grassetto,
corsivo, sottolineato, inverso). I colori sono richieste --- un nome o
`#rrggbb` --- e il front-end decide come approssimarli (@frame).

// ===========================================================================
= Il lavoro in background <sfondo>
// ===========================================================================

Tutto il lavoro che l'editor fa lontano dal thread dei comandi passa da un solo
oggetto, il #emph[WorkManager] di `work/`: la colorazione, la prescansione, il
controllo dei file, il salvataggio automatico, i lavori Lisp, i comandi di
shell e le ricerche nei file. Il WorkManager li esegue, li elenca, li ferma e
ne misura i tempi. Non conosce l'editor: lo stato su cui un lavoro lavora è un
parametro di tipo, `C`, che nell'editor è `EditorState<B>` e nei test di
`work/` è `()`.

#key[
  *Le regole, in sei righe.*
  + *Un solo punto d'ingresso.* Il lavoro asincrono passa dal WorkManager, ha
    un nome e un `WorkId`, e si può elencare e fermare.
  + *Due forme di lavoro.* I lavori a passi girano sull'esecutore; ciò che può
    bloccarsi, un processo o il disco, ha un thread dedicato.
  + *Un passo non aspetta mai.* Fa una quantità limitata di lavoro e dice come
    proseguire.
  + *Lo scheduler misura e segnala, non interrompe.* Un passo troppo lungo
    finisce nel log; la correzione si fa a mano.
  + *Gli argomenti non portano dati.* Dicono «vai a guardare»: la fonte della
    verità è lo stato.
  + *Tutto si prova senza thread.* Il nucleo riceve l'ora da un orologio che i
    test muovono a mano.
]

Le parole: nel codice un #emph[job] (lavoro) è ciò che gira a passi, e
#emph[work] è un lavoro o un thread dedicato; #emph[worker] resta solo nei nomi
del Lisp, come `define-worker` e `worker-fuel`. Restano fuori dalla prima
regola i thread che `(spawn ...)` avvia nell'interprete, e quello breve con cui
un comando di shell fermato riceve `KILL` (@utenti-sfondo).

== Il WorkManager <work-manager>

Ce n'è uno per editor, condiviso da ogni clone di `EditorState`: clonarlo costa
un `Arc`. La figura mostra che cosa possiede; in azzurro i thread, il resto
sono dati che qualunque thread raggiunge attraverso il WorkManager.

#canvas(height: 6.4cm)[
  #group(0cm, 0.35cm, 16.2cm, 5.9cm, [`EditorState`])
  #group(0.4cm, 1.0cm, 15.4cm, 5.0cm, [WorkManager --- uno per editor, condiviso dai cloni], colour: accent)

  #node(3.1cm, 1.95cm, 4.6cm, 0.8cm, [registro: nomi, lavori, thread], fill: paper)
  #node(7.65cm, 1.95cm, 3.7cm, 0.8cm, [opzioni], fill: paper)
  #node(12.55cm, 1.95cm, 5.7cm, 0.8cm, [argomenti: contatori degli eventi], fill: paper)

  #group(0.8cm, 3.0cm, 8.9cm, 2.65cm, [esecutore])
  #node(3.3cm, 4.35cm, 4.4cm, 1.5cm, [nucleo: \ code, scadenze, sottoscrizioni, regola, misure], fill: paper)
  #node(7.6cm, 4.35cm, 3.4cm, 1.5cm, [il thread \ `w1 executor`, \ o il test], fill: accent.lighten(85%))
  #carrow((5.9cm, 4.35cm), (5.5cm, 4.35cm))

  #node(12.55cm, 3.75cm, 5.7cm, 0.8cm, [thread dedicati: handle e `Stop`], fill: accent.lighten(85%))
  #node(12.55cm, 4.95cm, 5.7cm, 0.8cm, [watchdog: osserva, raccoglie, scrive il log], fill: accent.lighten(85%))
]

I file di `work/` seguono le parti:

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([File], [Che cosa contiene]),
  [`manager.rs`], [`WorkManager`, la facciata: costruzione, `start`, `submit`, `spawn`, `cancel`, le domande al registro, gli hook, `shutdown` e la modalità manuale; `Shared`, ciò che i suoi cloni condividono.],
  [`registry.rs`], [`Registry`: che cosa esiste, per numero e per nome (@registro).],
  [`executor.rs`], [`Msg`, `Executor` ed `executor_main`: la casella dei messaggi e il ciclo che guida il nucleo (@esecutore).],
  [`watchdog.rs`], [Il thread del watchdog e il suo turno (@watchdog).],
  [`core.rs`], [`Core`, il nucleo: i lavori, le code, la regola, le scadenze e le misure (@nucleo).],
  [`job.rs`], [Il contratto di un lavoro: `Job`, `Step`, `Exit`, `Wake`, `StepContext`, `Spec` e `WorkId` (@lavori).],
  [`topic.rs`], [`Topics` e `Topic`, gli argomenti (@argomenti).],
  [`threads.rs`], [I thread dedicati: `Stop`, `Lives` e `start_thread` (@thread-dedicati).],
  [`options.rs`, `report.rs`], [Le opzioni e le righe `[BKG]` del log (@misure).],
  [`clock.rs`], [`Clock`, `SystemClock` e `ManualClock` (@prove-sfondo).],
)

=== Due modalità

`WorkManager::threaded` è quello dell'editor: un thread esegue il nucleo
sull'orologio vero, da `start` a `shutdown`. `WorkManager::manual` è quello dei
test, e `editor/boot.rs` lo sceglie quando la libreria è compilata per i test
(`cfg!(test)`): nulla gira da solo, il nucleo legge un `ManualClock`, e il
lavoro avanza quando il test chiama `run_until_idle` o `run_for`
(@prove-sfondo). Il resto è uguale nelle due modalità, compreso il modo in cui
entra un lavoro.

Una chiamata al WorkManager non tocca mai il nucleo. Cambia il registro, che
risponde subito, e lascia un messaggio nella casella dell'esecutore, che lo
legge tra un passo e l'altro. Così `submit` dall'interno di un passo è sicuro
come da qualunque altro punto, anche in modalità manuale, dove l'esecutore è
il thread del test.

=== Il ciclo di vita

+ *Nasce* in `EditorState::new`, prima dei compartimenti, perché i buffer
  ricevono da lui l'argomento `buffer-changed` (@argomenti). Il `Log`
  dell'editor nasce ancora prima: le segnalazioni del WorkManager vanno lì.
+ *Parte* con `start(stato)`, appena lo stato esiste: il thread dell'esecutore
  riceve la sua copia di `EditorState`, e con lui parte il watchdog. Ciò che è
  stato sottomesso prima aspetta nella casella. In modalità manuale `start`
  non fa nulla.
+ *Riceve i lavori dell'editor*: evidenziatore e prescansione in
  `EditorState::new`, controllo dei file e salvataggio automatico in
  `create_global_env`, dopo `init.lisp`, perché il loro intervallo è
  un'impostazione (@utenti-sfondo).
+ *Termina* con `shutdown`, che l'editor espone come
  `EditorState::shutdown(entro)`. Il front-end lo chiama all'uscita, con una
  guardia che lo chiama anche quando un panic attraversa `main`
  (`ShutDownOnExit` in `src/main.rs`, con mezzo secondo di attesa).

`shutdown` procede così:

+ chiude il registro: da qui `submit` e `spawn` rispondono `None`;
+ mette `Msg::Shutdown` nella casella con il lock del registro preso, così ogni
  lavoro accettato prima è già nella casella;
+ alza lo `Stop` di ogni thread dedicato ed esegue la sua azione di sblocco;
+ se nessun thread ha preso l'esecutore, in modalità manuale o prima di
  `start`, lo chiude sul thread del chiamante, e le `exit` dei lavori ricevono
  lo stato passato a `shutdown`; altrimenti lo chiude il thread
  dell'esecutore quando legge il messaggio. Ogni lavoro esce con
  `Exit::Shutdown`;
+ sveglia il watchdog, che si ferma, e aspetta che tutti i thread siano
  finiti, al massimo per `entro`;
+ fa `join` di ciò che è finito e abbandona il resto, nominandolo nel log: un
  thread che non si ferma, un passo che non finisce;
+ chiama un'ultima volta l'hook del watchdog, che porta sul file le righe
  appena scritte.

Una seconda chiamata non fa nulla.

=== Chi parla con chi

#canvas(height: 7.4cm)[
  #lane(2.0cm, 0cm, 7.3cm, [thread dei comandi], fill: accent.lighten(85%), w: 3.2cm)
  #lane(6.6cm, 0cm, 7.3cm, [esecutore], fill: accent.lighten(85%), w: 2.6cm)
  #lane(10.9cm, 0cm, 7.3cm, [thread dedicato], fill: accent.lighten(85%), w: 2.8cm)
  #lane(14.6cm, 0cm, 7.3cm, [watchdog], fill: accent.lighten(85%), w: 2.4cm)

  #msg(2.0cm, 6.6cm, 1.2cm, [`submit(spec, lavoro)`], colour: accent)
  #self-msg(2.0cm, 1.65cm, [tasto → `emit`])
  #msg(2.0cm, 6.6cm, 2.55cm, [`Msg::Nudge`, uno per raffica], dash: "dashed")
  #self-msg(6.6cm, 2.95cm, [+20 ms: un passo])
  #msg(6.6cm, 2.0cm, 3.85cm, [callback dovuta (lavoro Lisp)])
  #msg(2.0cm, 10.9cm, 4.55cm, [`spawn("*Shell Output*", corpo)`], colour: accent)
  #msg(2.0cm, 10.9cm, 5.2cm, [`cancel`: `Stop` e azione di sblocco])
  #msg(10.9cm, 14.6cm, 5.85cm, [finito: `join`, un panic nel log], dash: "dashed")
  #msg(14.6cm, 6.6cm, 6.5cm, [osserva il passo in corso], dash: "dashed", colour: accent)
]

- *Il thread dei comandi* chiama il WorkManager: sottomette lavori, avvia
  thread dedicati, ferma, cambia opzioni. Le porte delle modifiche emettono gli
  argomenti, e una raffica di emissioni manda all'esecutore una sola sveglia.
  Le chiamate non aspettano: girano sul thread di chi chiama.
- *L'esecutore* esegue i passi uno alla volta. Un lavoro Lisp non chiama il
  suo modo: lascia una callback dovuta, che il thread dei comandi esegue tra un
  comando e l'altro (@utenti-sfondo).
- *Un thread dedicato* lavora per conto suo, e si ferma con lo `Stop` e con
  l'azione che lo sblocca.
- *Il watchdog* osserva il passo in corso, raccoglie i thread finiti e scrive
  il file di log, così una segnalazione arriva sul file anche quando ciò che
  segnala è bloccato.

== I lavori <lavori>

Un lavoro implementa `Job<C>`:

#estratto("work/job.rs")[
```rust
pub trait Job<C>: Send + 'static {
    fn step(&mut self, cx: &mut StepContext<'_, C>) -> Result<Step, JobError>;
    fn exit(&mut self, _state: &C, _why: Exit) {}
}
```
]

Lo `StepContext` porta lo stato (`state`), il motivo del passo (`woken_by`:
`Start`, `Turn`, `Event` o `Timer`) e il numero del lavoro (`id`). Con
`cx.worked(unità, su_che_cosa)` il passo dichiara quanto ha fatto, e con
`cx.subscribe(argomenti)` cambia gli argomenti che lo svegliano. Il passo
risponde con un `Step`:

#ref-table(
  columns: (3.0cm, 1fr),
  header: ([`Step`], [Quando viene il passo successivo]),
  [`Continue`], [c'è altro da fare: il lavoro torna in fondo alla sua coda, e se nessun altro è pronto riparte subito],
  [`Sleep(d)`], [dopo `d`; gli eventi nel frattempo non lo svegliano, ma il passo dopo vede lo stato che annunciavano],
  [`Wait`], [al prossimo evento di uno dei suoi argomenti],
  [`WaitAtMost(d)`], [al prossimo evento, o dopo `d` al più tardi],
  [`Done`], [mai: il lavoro ha finito],
)

Le regole di un passo:

- *Non aspetta mai.* Niente I/O che può bloccarsi, che va su un thread
  dedicato, e niente `sleep`: chi deve aspettare lo dice nell'esito.
- *Non tiene lock da un passo all'altro.* Copia sotto lock, lavora senza, e
  salva solo se la versione non è cambiata.
- *Non guarda l'orologio per decidere quanto fare.* Divide il lavoro in unità
  dei suoi dati (righe, file, carburante) e le dichiara con `cx.worked`; il
  tempo lo misura lo scheduler.
- *Non si interrompe.* `cancel` ha effetto alla fine del passo in corso.
- *Gli errori passano dal `Result`.* Lo scheduler li scrive nel log con il nome
  del lavoro, e il lavoro esce.

Un lavoro sottomesso è pronto, o in attesa della sua scadenza se la `Spec` dice
`Start::After`. Poi passa da uno stato all'altro così:

#canvas(height: 5.3cm)[
  #node(2.0cm, 1.6cm, 2.6cm, 0.8cm, [pronto], fill: paper)
  #node(7.0cm, 1.6cm, 3.0cm, 1.0cm, [in corso \ (un passo)], fill: accent.lighten(85%))
  #group(10.3cm, 0.35cm, 5.9cm, 3.0cm, [in attesa])
  #node(13.25cm, 0.95cm, 5.0cm, 0.62cm, [di una scadenza: `Sleep`], fill: paper)
  #node(13.25cm, 1.8cm, 5.0cm, 0.62cm, [di un evento: `Wait`], fill: paper)
  #node(13.25cm, 2.65cm, 5.0cm, 0.62cm, [dell'uno o dell'altro: `WaitAtMost`], fill: paper)
  #node(7.0cm, 3.95cm, 2.6cm, 0.7cm, [uscito: `exit`], fill: paper)

  #carrow((3.3cm, 1.42cm), (5.5cm, 1.42cm), label: [scelto], dy: -0.27cm)
  #carrow((5.5cm, 1.8cm), (3.3cm, 1.8cm), label: [`Continue`], dy: 0.27cm)
  #carrow((8.5cm, 1.6cm), (10.3cm, 1.6cm), label: [attende], dy: -0.27cm)
  #carrow((7.0cm, 2.1cm), (7.0cm, 3.6cm), label: [`Done`, errore, panic], dx: 1.25cm)

  #segment((13.25cm, 3.35cm), (13.25cm, 4.75cm), colour: accent, dash: "dashed")
  #segment((13.25cm, 4.75cm), (2.0cm, 4.75cm), colour: accent, dash: "dashed")
  #arrow((2.0cm, 4.75cm), (2.0cm, 2.0cm), colour: accent, dash: "dashed")
  #note(2.3cm, 4.85cm, colour: accent)[la scadenza o l'evento lo riportano «pronto»]
]

`cancel` e `shutdown` portano a «uscito» da qualunque stato; un lavoro in corso
ci arriva alla fine del passo. Il motivo dell'uscita è un `Exit`:

#ref-table(
  columns: (3.0cm, 1fr),
  header: ([`Exit`], [Quando]),
  [`Finished`], [un passo ha risposto `Done`],
  [`Failed`], [un passo ha restituito un errore, o ha risposto `Wait` senza un argomento che possa svegliarlo],
  [`Panicked`], [un passo è andato in panic: il nucleo lo intercetta con `catch_unwind`],
  [`Cancelled`], [qualcuno l'ha fermato, o un lavoro con lo stesso nome l'ha sostituito],
  [`Shutdown`], [il WorkManager si chiude],
)

`exit` arriva una volta sola, per qualunque motivo, anche a un lavoro fermato
prima del primo passo: è il posto della pulizia, per esempio la callback
`ON-DONE` di un lavoro Lisp. Gira sull'esecutore come un passo, ed è protetta e
osservata come un passo.

Un lavoro si sottomette con una `Spec`, che ne dà il nome e il modo di
eseguirlo: `Spec::new(nome)`, poi `class`, `unit` (l'unità di `cx.worked`, come
le segnalazioni la stampano), `on` (un argomento) e `start`. Il nome è unico:
sottomettere un lavoro con un nome già preso ferma ciò che lo porta, lavoro o
thread dedicato. Il `WorkId` è un numero di tutto il processo e non si riusa:
un numero tenuto dopo la fine del suo lavoro non può nominarne un altro.

== Il nucleo <nucleo>

`Core` tiene i lavori e decide che cosa gira. Non ha un thread né un orologio
suo: chi lo guida chiama `run_once` un turno alla volta, e il nucleo legge l'ora
dal `Clock` che ha ricevuto. Un lavoro è in un solo posto alla volta (`Place`):
pronto, nella coda della sua classe; in corso; in attesa di una scadenza; in
attesa di un argomento, e forse anche di una scadenza. Un turno:

+ chiama `exit` sui lavori usciti, con lo stato che il turno porta;
+ rende pronti i lavori la cui scadenza è passata o i cui eventi sono maturi
  (`poll`), nell'ordine in cui lo sono diventati;
+ chiude le finestre di segnalazione scadute e controlla il carico
  dell'ultimo secondo (`tick`, @misure);
+ sceglie un lavoro pronto (`choose`) ed esegue un suo passo, misurato
  (`run_step`);
+ chiama `exit` sui lavori che il passo ha fatto uscire.

Mentre fa il passo, il lavoro è fuori dal nucleo: il suo record non lo
contiene, e un lavoro non può girare due volte insieme. Un lavoro fermato
durante il passo trova il record sparito quando il passo finisce, ed esce
allora. Mentre un passo o una `exit` è in corso, il nucleo lo scrive in
`Underway` (il lavoro, l'inizio e un numero d'ordine), che altri thread
leggono: il watchdog, per segnalare un passo che non finisce, e
`WorkManager::jobs`, per elencare il lavoro come in corso. È proprio quando un
passo non finisce che il nucleo non può rispondere da sé.

=== Chi passa prima

Le classi sono due. `Class::High` è per ciò che l'utente aspetta di vedere,
colore e prescansione; `Class::Normal` è il default, e vi stanno il controllo
dei file, il salvataggio automatico e tutti i lavori Lisp. Ogni classe ha la sua
coda, e un lavoro entra in fondo alla sua ogni volta che diventa pronto. Passa
prima la classe alta, ma dopo `high-steps` passi alti di fila (4 per default)
tocca a un lavoro normale pronto. La regola si conta in passi e non in tempo,
così è deterministica.

#canvas(height: 1.7cm)[
  #for (i, c) in "HHHHNHHHHNHHHHN".clusters().enumerate() {
    node(0.55cm + i * 1.08cm, 0.55cm, 0.86cm, 0.6cm,
      if c == "H" [`E`] else [`I`],
      fill: if c == "H" { accent.lighten(85%) } else { paper })
  }
  #note(0.2cm, 1.05cm)[`E` = l'evidenziatore su un file enorme, che risponde sempre `Continue`; `I` = un lavoro Lisp con `(yield 'continue)`: un passo ogni cinque.]
]

Quando l'evidenziatore ha finito, il lavoro Lisp ha l'esecutore tutto per sé.

=== Le scadenze

Un lavoro ha al più una scadenza, messa da `Sleep`, da `WaitAtMost` o da
`Start::After`, e contata dalla fine del passo: un lavoro lento non si accoda a
sé stesso, e un periodo saltato non si recupera. Nessuna scadenza va oltre un
anno (`FURTHEST`): un
`Instant` molto più lontano può non esistere, e sommarlo andrebbe in panic
sull'esecutore, fuori da ogni passo.

== Gli argomenti <argomenti>

Un argomento (`Topic`) è un evento con un nome, e non porta dati: è un
contatore. Emetterlo (`emit`) costa un incremento atomico e, se l'esecutore non
ha già una sveglia in arrivo, un `Msg::Nudge` nella sua casella; si può fare
anche con il lock di un buffer preso. Un nome si risolve una volta
(`WorkManager::topic`), e da lì l'argomento si emette senza toccare lock.

- *La sottoscrizione è persistente.* Un lavoro ha eventi in sospeso quando il
  contatore di un suo argomento ha superato l'ultimo valore che ha visto. Il
  nucleo aggiorna quel valore #emph[prima] del passo: un evento che arriva
  durante il passo resta non visto, e risveglia il lavoro.
- *Il ritardo raccoglie le raffiche.* Un lavoro in attesa diventa pronto
  `event-delay` (20 ms) dopo che il nucleo ha notato il primo evento non visto,
  e gli eventi che arrivano nel frattempo si sommano a quello. Gli eventi
  arrivati durante un passo maturano dalla fine del passo.
- *Un `Wait` senza argomenti è un errore.* Nessuno potrebbe svegliare il
  lavoro: il nucleo lo segnala, e il lavoro esce con `Exit::Failed`.
- *Un passo può cambiare i suoi argomenti* con `cx.subscribe`: è ciò a cui si
  riduce un `(yield '(wait buffer-changed))` del Lisp. Un argomento nuovo
  conta come cambiato, perché il passo che lo nomina può aver guardato lo stato
  appena prima di un'emissione.
- *Un lavoro che dorme ignora gli eventi*, senza perderli: il passo successivo
  vede lo stato che annunciavano.

Un esempio: tre tasti in 15 ms producono un solo passo, e un quarto tasto che
arriva durante il passo ne produce un secondo. Le frecce sono le emissioni, con
il valore del contatore.

#canvas(height: 2.85cm)[
  // Asse del tempo: 0,28 cm per millisecondo, da 0 a 50 ms.
  #let t(ms) = 1.0cm + ms * 0.28cm
  #arrow((t(0), 2.2cm), (t(52), 2.2cm), colour: dim)
  #for ms in (0, 10, 20, 30, 40, 50) {
    segment((t(ms), 2.12cm), (t(ms), 2.28cm), colour: dim)
    note(t(ms) - 0.2cm, 2.35cm)[#ms ms]
  }
  // Le emissioni, con il valore che il contatore assume.
  #for (ms, n) in ((0, 1), (5, 2), (10, 3), (23, 4)) {
    arrow((t(ms), 1.5cm), (t(ms), 2.15cm), colour: warm)
    note(t(ms) + 0.08cm, 1.45cm, colour: warm)[#n]
  }
  // Il ritardo: dal primo evento non visto, poi dalla fine del passo.
  #segment((t(0), 1.2cm), (t(20), 1.2cm), colour: accent, dash: "dashed")
  #segment((t(26), 1.2cm), (t(46), 1.2cm), colour: accent, dash: "dashed")
  #note(t(1), 0.82cm, colour: accent)[`event-delay`]
  // I passi.
  #note(t(20) - 0.1cm, 0.0cm)[visto = 3]
  #node(t(23), 0.62cm, 6 * 0.28cm, 0.42cm, [passo], fill: accent.lighten(85%), size: 7pt)
  #note(t(46) - 2.3cm, 0.0cm)[4 > 3: di nuovo]
  #node(t(48), 0.62cm, 4 * 0.28cm, 0.42cm, [passo], fill: accent.lighten(85%), size: 7pt)
]

L'editor emette due argomenti, dichiarati in `editor/work.rs`; il Lisp ne
emette di suoi con `emit-event`.

#ref-table(
  columns: (3.3cm, 1fr),
  header: ([Argomento], [Chi lo emette]),
  [`buffer-changed`], [`edits::changed` (le due porte, undo, redo, `replace-back`), `Buffer::adopt_text`, il cambio di modo maggiore, un buffer che entra nella tabella],
  [`syntax-changed`], [`set_mode`, `edit_grammar`, `edit_syntax_table`],
)

Il modulo dei buffer non conosce lo scheduler. Il compartimento `Buffers` riceve
l'argomento `buffer-changed` già risolto e lo dà a ogni buffer che entra nella
tabella; fino ad allora un buffer ha un argomento staccato
(`Topic::detached`), che conta e non sveglia nessuno.

== L'esecutore <esecutore>

L'esecutore (`Executor`) è il nucleo con l'altro capo della casella. Lo guida il
thread `w1 executor` dell'editor, da `start` a `shutdown` (`executor_main`), o
il test in modalità manuale; il numero nel nome dei thread è quello del
WorkManager nel processo. Un turno applica i messaggi in attesa e poi esegue al
più un passo:

#estratto("work/executor.rs")[
```rust
pub(super) fn turn(&mut self, shared: &Shared<C>, state: &C) -> bool {
    self.drain();
    if self.shut {
        return false;
    }
    shared.nudged.swap(false, Ordering::AcqRel);
    let stepped = self.core.run_once(state);
    self.settle(shared);
    if stepped {
        shared.stepped();
    }
    stepped
}
```
]

Il ciclo di `executor_main` ripete i turni finché un passo va, e dorme quando
non c'è niente di pronto:

#canvas(height: 6.4cm)[
  #node(7.2cm, 0.6cm, 7.4cm, 0.8cm, [*1* #h(0.3em) dorme fino alla prossima scadenza o a un messaggio])
  #node(7.2cm, 1.85cm, 7.4cm, 0.8cm, [*2* #h(0.3em) applica i messaggi: `Submit`, `Cancel`, `Nudge`, `Shutdown`])
  #node(7.2cm, 3.1cm, 7.4cm, 0.8cm, [*3* #h(0.3em) `run_once`: uscite, scadenze ed eventi maturi, scelta])
  #node(7.2cm, 4.35cm, 7.4cm, 0.8cm, [*4* #h(0.3em) un passo: `catch_unwind`, misura, esito], fill: accent.lighten(85%))
  #node(7.2cm, 5.6cm, 7.4cm, 0.8cm, [*5* #h(0.3em) righe nel log, uscite dal registro, `after_step`])

  #carrow((7.2cm, 1.0cm), (7.2cm, 1.45cm))
  #carrow((7.2cm, 2.25cm), (7.2cm, 2.7cm))
  #carrow((7.2cm, 3.5cm), (7.2cm, 3.95cm))
  #carrow((7.2cm, 4.75cm), (7.2cm, 5.2cm))

  #segment((10.9cm, 5.6cm), (11.6cm, 5.6cm))
  #segment((11.6cm, 5.6cm), (11.6cm, 1.85cm))
  #arrow((11.6cm, 1.85cm), (10.9cm, 1.85cm))
  #note(11.8cm, 3.1cm)[un passo è andato: \ subito al turno dopo, \ le misure ogni 100 ms]

  #segment((3.5cm, 3.1cm), (2.8cm, 3.1cm))
  #segment((2.8cm, 3.1cm), (2.8cm, 0.6cm))
  #arrow((2.8cm, 0.6cm), (3.5cm, 0.6cm))
  #note(0.0cm, 1.45cm)[niente di pronto: \ pubblica le misure \ e torna a dormire]
]

- *Le sveglie non si perdono.* Il flag `nudged` dice che un `Msg::Nudge` è già
  nella casella, e fa di una raffica di emissioni una sola sveglia. Il turno lo
  abbassa con uno scambio atomico #emph[prima] che il nucleo guardi gli
  argomenti: un'emissione che il nucleo non vedrà trova il flag basso e manda
  una sveglia, e una che lo trova alto è avvenuta prima, e il nucleo la vede.
- *Dopo ogni passo, l'hook.* Quando il registro ha già tolto i lavori usciti,
  il turno chiama l'hook di `after_step`: l'editor lo usa per dire al front-end
  che lo schermo può essere cambiato (@utenti-sfondo). L'hook gira tra due
  passi, quindi è breve e non aspetta; un suo panic va nel log.
- *Le misure si pubblicano.* Le misure dei lavori passano dal nucleo al
  registro ogni `PUBLISH_EVERY` (100 ms) mentre l'esecutore è occupato, e
  prima di dormire quando non lo è; in modalità manuale, alla fine di ogni
  guida.
- *Dorme fino alla prossima cosa da fare*: la scadenza più vicina, un evento che
  matura o la chiusura di una finestra di segnalazione (`next_wakeup`), oppure
  un messaggio, se arriva prima.

== Il registro <registro>

Il registro (`Registry`, dietro il lock `registry` di `Shared`) è la risposta
del WorkManager a «che cosa esiste», data subito e senza chiedere
all'esecutore:

#ref-table(
  columns: (2.4cm, 1fr),
  header: ([Campo], [Che cosa tiene]),
  [`closed`], [vero dopo `shutdown`: da lì non entra più nulla],
  [`names`], [un solo spazio di nomi per lavori e thread: un nome indica una cosa sola, e fermarlo per nome ferma quella, qualunque sia],
  [`latest`], [il lavoro più recente sottomesso con ogni nome, finché qualcuno non lo ferma; resta anche dopo la sua fine],
  [`jobs`], [i lavori, con ciò che l'esecutore ha pubblicato di loro (`WorkInfo`)],
  [`threads`], [i thread dedicati (`Dedicated`): nome, `Stop`, handle, avvio, richiesta di stop],
)

I messaggi per l'esecutore, tranne `Msg::Nudge`, partono con il lock del
registro preso: la casella li tiene nell'ordine in cui il registro è cambiato,
e dopo `Msg::Shutdown` non ne arriva un altro. Togliere un lavoro dal registro
(`withdraw`) lascia un `Withdrawn` da completare senza lock: l'azione di
sblocco di un thread gira dopo, sul thread di chi ha chiamato `cancel`,
protetta come un passo.

Le domande che il registro risponde:

- `jobs()` elenca i lavori nell'ordine di sottomissione, con le misure
  dell'ultima pubblicazione; un lavoro che l'esecutore non ha ancora visto
  appare come è stato sottomesso. Il lavoro nel mezzo di un passo è elencato
  come in corso da ciò che dice `Underway`, perché dopo un passo che non
  finisce non si pubblica più nulla.
- `threads()` elenca i thread dedicati ancora vivi, dopo aver raccolto quelli
  finiti.
- `is_named(nome)` dice se un nome è preso. Lo usa `free_name_for_work`: un
  comando di shell prende il nome del buffer in cui scrive, e quel nome deve
  essere libero sia tra i buffer sia qui.
- `is_latest(nome, id)` dice se `id` parla ancora per il suo nome. È la domanda
  delle callback dovute (@utenti-sfondo): un lavoro finito ha ancora diritto
  alla sua `ON-DONE`, uno sostituito o fermato no.

== I thread dedicati <thread-dedicati>

Un thread dedicato serve a ciò che può bloccarsi e non si divide in passi:
leggere l'output di un processo, percorrere un albero di directory.
`WorkManager::spawn(nome, corpo)` lo avvia con il nome `w1 nome` (Linux ne
mostra 15 byte), lo mette nel registro accanto ai lavori e restituisce il suo
`WorkId`; il corpo riceve lo `Stop` e il proprio numero.

#canvas(height: 3.7cm)[
  #node(2.3cm, 1.0cm, 3.0cm, 0.8cm, [in corso], fill: accent.lighten(85%))
  #node(13.2cm, 1.0cm, 3.0cm, 0.8cm, [finito: `join`], fill: paper)
  #node(7.7cm, 2.7cm, 3.0cm, 0.8cm, [in chiusura], fill: accent.lighten(85%))
  #node(13.2cm, 2.7cm, 3.0cm, 0.8cm, [abbandonato], fill: paper)

  #carrow((3.8cm, 1.0cm), (11.7cm, 1.0cm), label: [il corpo ritorna], dy: -0.25cm)
  #carrow((3.2cm, 1.4cm), (6.2cm, 2.5cm), label: [`cancel`: `Stop` e azione], dx: -1.6cm, dy: 0.35cm)
  #carrow((9.2cm, 2.5cm), (12.2cm, 1.4cm), label: [si ferma], dx: -0.3cm, dy: -0.35cm)
  #carrow((9.2cm, 2.7cm), (11.7cm, 2.7cm), label: [allo `shutdown`], dy: 0.3cm)
  #note(5.9cm, 3.2cm, colour: warm)[dopo `stuck-step`: segnalato]
]

- *Un thread non si uccide: gli si chiede di fermarsi.* `cancel` alza lo
  `Stop`, che il corpo controlla nei punti da cui può uscire, ed esegue
  l'azione registrata con `Stop::on_cancel`, che sblocca ciò su cui il corpo
  aspetta: per un comando di shell, fermare il processo chiude la pipe che il
  corpo legge. L'azione gira una volta sola, anche se è registrata dopo la
  richiesta, e gira sul thread che chiede lo stop: deve essere breve.
- *Il `join` non blocca nessuno.* Il watchdog, e chi chiede l'elenco dei
  thread, lo fa solo sui thread che risultano finiti (`is_finished`); un panic
  nel corpo torna dal `join` e va nel log.
- *Un thread che non si ferma*, per esempio una `stat` su un disco di rete
  appeso, resta elencato come in chiusura, viene segnalato dopo `stuck-step` e,
  allo `shutdown`, abbandonato.
- *`shutdown` sa quanto aspettare.* Ogni thread del WorkManager porta una quota
  di `Lives` dall'avvio alla fine; `shutdown` aspetta che il conto torni a
  zero, al più per il tempo che gli è dato.

L'editor non chiama `spawn` direttamente. `EditorState::spawn_work` passa al
corpo l'editor, tiene il thread fuori dal thread dei comandi per tutta la sua
vita (`WorkerTurn`, @utenti-sfondo) e, quando il corpo ritorna, dice al
front-end di ridisegnare. `spawn_writer` aggiunge un buffer di uscita: il
thread prende il nome del buffer, il buffer sa chi scrive in lui prima che il
corpo possa scrivere una riga, e chiudere il buffer ferma il thread.

== Il watchdog <watchdog>

Il watchdog è un thread, `w1 watchdog`, che parte con l'esecutore, o con il
primo thread dedicato se viene prima, e si sveglia ogni `watchdog-every`
(250 ms). Guarda ciò che nessun altro può guardare: il passo in corso, che
l'esecutore non può segnalare finché ci sta dentro. Un turno (`watch_turn`):

+ raccoglie i thread dedicati finiti (`reap`), e segnala quelli ancora vivi
  `stuck-step` dopo la richiesta di stop;
+ segnala il passo in corso se dura da `stuck-step` (1 s), e di nuovo ogni volta
  che la sua durata raddoppia: 1 s, 2 s, 4 s… Un turno in ritardo segnala una
  volta sola, con la durata vera, e il segno successivo è il primo raddoppio
  oltre quella. Il numero d'ordine di `Underway` distingue un passo dal
  successivo, anche dello stesso lavoro; una `exit` che non finisce è
  segnalata allo stesso modo, «in its exit»;
+ chiama per ultimo l'hook di `after_watch`, così ciò che il turno ha
  segnalato è già in ciò che l'hook scrive.

L'editor passa ad `after_watch` la funzione `Log::flush`: il file di log si
scrive qui, e non da chi scrive una riga. In modalità manuale il watchdog non
c'è: i thread finiti si raccolgono quando il test guida il WorkManager, e il
test fa un turno del watchdog con `watch_once`.

== Misure, segnalazioni e opzioni <misure>

Il nucleo misura ogni passo: quanto è durato, quanto il lavoro ha aspettato in
coda e le unità dichiarate con `cx.worked`, da cui viene il costo per unità.
Per ogni lavoro tiene i passi, il tempo totale e il passo più lungo; per
l'esecutore, l'occupazione dell'ultimo secondo, misurata esattamente su una
finestra che scorre. Quando una misura supera la sua soglia, una riga `[BKG]`
va nel log. Nessuna segnalazione cambia ciò che un lavoro fa: un lavoro lento si
corregge a mano, con le segnalazioni come prova.

#ref-table(
  columns: (2.6cm, 5.6cm, 1fr),
  header: ([Riga], [Quando (opzione)], [Che cosa riporta]),
  [`long step`], [un passo dura più di `long-step`], [lavoro, durata, su che cosa, unità e costo per unità],
  [`late start`], [un lavoro pronto aspetta più di `late-high` o di `late-normal`], [di quanto, e dietro quale lavoro],
  [`load`], [l'esecutore è occupato per più di `busy-share` dell'ultimo secondo], [la quota, e i tre lavori che la causano],
  [`summary`], [si chiude una finestra di `summary-every` che ha contato qualcosa], [quante altre occorrenze, e la peggiore],
  [`stuck step`], [un passo o una `exit` è in corso da `stuck-step`, poi dal doppio], [lavoro e durata; la scrive il watchdog],
  [`thread`], [un thread dedicato è vivo `stuck-step` dopo la richiesta di stop, o viene abbandonato allo `shutdown`], [nome e tempo trascorso],
  [`error`, `panic`], [sempre], [lavoro e messaggio; il lavoro esce],
)

La soglia del ritardo cambia con la classe perché un lavoro normale, per
progetto, aspetta fino a `high-steps` passi alti. Per ogni coppia di lavoro e
regola la prima occorrenza va subito nel log e apre una finestra di
`summary-every`; le successive nella finestra si contano soltanto, e alla
chiusura un riepilogo dice quante erano e la peggiore. Una finestra che si
chiude vuota finisce, e la prima occorrenza dopo un periodo tranquillo va di
nuovo subito nel log. Errori e panic non hanno limite: il lavoro esce, e non
può ripeterli.

#block(breakable: false)[
```text
[BKG] long step: highlighter, 912 ms on app.min.js (3 lines, 304 ms each)
[BKG] late start: file-watcher started 230 ms late, behind highlighter (6 steps)
[BKG] load: 64% of the last second (highlighter 41%, manpage-index 20%)
[BKG] summary: highlighter, 12 more long steps in the last 10 s (worst 912 ms)
[BKG] stuck step: manpage-index running for 2.0 s
[BKG] thread: *Shell Output* still running 1000 ms after it was asked to stop
```
]

Le opzioni stanno nel WorkManager, in valori atomici che il nucleo legge a ogni
scelta: una modifica vale dal passo successivo, e chi la fa non aspetta lo
scheduler. Il Lisp le legge con `work-option` e `work-options` e le cambia con
`set-work-option`, che rifiuta un valore fuori dall'intervallo. I tempi sono in
millisecondi.

#ref-table(
  columns: (3.4cm, 1.7cm, 2.4cm, 1fr),
  header: ([Opzione], [Default], [Valori], [Che cosa regola]),
  [`high-steps`], [4], [1--1000], [passi alti di fila prima di uno normale],
  [`event-delay`], [20], [0--60 000], [il ritardo dopo il primo evento non visto],
  [`long-step`], [40], [1--3 600 000], [la soglia del passo lungo],
  [`late-high`], [40], [1--3 600 000], [la soglia del ritardo, classe alta],
  [`late-normal`], [200], [1--3 600 000], [la soglia del ritardo, classe normale],
  [`stuck-step`], [1000], [1--3 600 000], [la soglia del passo bloccato e del thread che non si ferma],
  [`busy-share`], [50 %], [1--100], [la soglia del carico],
  [`summary-every`], [10 000], [1--3 600 000], [la durata di una finestra di riepilogo],
  [`watchdog-every`], [250], [1--60 000], [ogni quanto si sveglia il watchdog],
)

`work-list` elenca ciò che è in background, lavori e thread, con stato e
misure. La pagina `background` del manuale dell'utente spiega le une e le
altre a chi scrive in Lisp.

== Gli utenti del WorkManager <utenti-sfondo>

=== I lavori dell'editor

#ref-table(
  columns: (2.8cm, 1.8cm, 4.4cm, 1fr),
  header: ([Lavoro], [Classe, unità], [Quando gira], [Che cosa fa a ogni passo]),
  [`highlighter`], [alta, righe], [a `buffer-changed` e `syntax-changed`: `Continue` finché un buffer è indietro, poi `Wait`], [colora fino a 500 righe del primo buffer indietro, partendo da quello con il focus],
  [`prescanner`], [alta, righe], [come l'evidenziatore], [porta avanti di fino a 500 righe la `ScanCache` di un buffer, con una fotografia ogni 64 (@modi)],
  [`file-watcher`], [normale, buffer], [`watch-file-interval` (3 s) dopo l'avvio e dopo ogni passo], [confronta con il disco al più 8 buffer; ricarica o segna],
  [`auto-saver`], [normale, file], [`auto-save-interval` (30 s) dopo l'avvio e dopo ogni passo], [scrive la copia di al più 4 buffer modificati],
)

Il codice è in `modes/`: `highlighter.rs`, `prescan.rs`, `watcher.rs` e
`autosave.rs`.

Evidenziatore e prescansione sono sottomessi in `EditorState::new`: un lavoro
che aspetta un cambiamento non costa nulla. Controllo dei file e salvataggio
automatico sono sottomessi in `create_global_env`, dopo `init.lisp`, perché
l'interruttore che li spegne e il loro intervallo sono impostazioni del Lisp,
che rileggono a ogni passo. Ognuno è sottomesso una volta sola: un lavoro
dell'editor che va in panic non torna fino alla fine della sessione
#rapporto("BKG-2").

Il passo dell'evidenziatore è il modello per scriverne altri:

#estratto("modes/highlighter.rs")[
```rust
impl<B: BufferTrait> Job<EditorState<B>> for Highlighter {
    fn step(&mut self, cx: &mut StepContext<'_, EditorState<B>>) -> Result<Step, JobError> {
        match cx.state.highlight_one_turn() {
            Some((buffer, lines)) => {
                cx.worked(lines as u64, &buffer);
                Ok(Step::Continue)
            }
            None => Ok(Step::Wait),
        }
    }
}
```
]

`highlight_one_turn` copia sotto lock le righe da colorare, lo stato del
lessico all'inizio della prima e la versione del buffer; colora senza lock;
riprende il lock e salva solo se la versione e il modo sono ancora quelli
copiati. Un carattere digitato nel frattempo rende inutile il turno, che il
passo successivo rifà dalla riga cambiata. Lo stesso schema regge la
prescansione, che però legge le righe sotto il lock di lettura perché la
scansione è veloce, il controllo dei file e il salvataggio automatico. Il costo
di una riga
cresce più che linearmente con la sua lunghezza #rapporto("SIN-2"), e una
regione di sintassi che corrisponde alla stringa vuota fa un passo che non
finisce: il watchdog lo segnala, ma l'esecutore resta fermo #rapporto("SIN-1").

=== I comandi di shell e le ricerche <comandi-shell>

`shell-command-start` (`primitives/shell.rs`) e la ricerca nei file
(`grep--scan`, in `primitives/results.rs`) girano su un thread dedicato avviato
con `spawn_writer`, con il nome del buffer in cui scrivono: `*Shell Output*`, o
`<2>`, `<3>`… se il nome è preso tra i buffer o tra i lavori
(`free_name_for_work`). Il comando passa dalla shell della piattaforma
(`/bin/sh -c`, o `cmd /C`), con l'input vuoto e stdout e stderr in una sola
pipe, che il thread legge come byte: ciò che arriva va nel buffer subito, anche
una riga non ancora finita, con ciò che non è UTF-8 sostituito e un carattere
diviso tra due letture tenuto per la successiva. Alla fine il thread aspetta il
processo e aggiunge una riga `--- exited N ---`, o `--- killed ---`.

Lo ferma `stop-worker` con il nome del suo buffer, o la chiusura del buffer. Il
comando gira in un gruppo di processi suo, e la richiesta raggiunge tutto ciò
che ha avviato: su Unix l'azione di sblocco manda `TERM` e `CONT` al gruppo,
con il `kill` della shell, perché la libreria standard segnala un processo e
non un gruppo, e un thread breve, `stop shell`, avviato fuori dal WorkManager,
manda `KILL` due secondi dopo a ciò che resta. All'uscita dell'editor quel
`KILL` non arriva, e un comando che ignora `TERM` resta vivo
#rapporto("SHL-5"). Ciò che ha lasciato il gruppo, come un demone, non viene
raggiunto, e se tiene l'output che ha ricevuto la lettura resta aperta: la riga
finale aspetta che lo lasci. `shell-command-to-string`, invece, aspetta il comando sul thread che
lo chiama e ne restituisce lo stdout. La pagina `shell` del manuale dell'utente
dice che cosa vede chi li usa.

=== I lavori Lisp <lavori-lisp>

`define-worker` e `background-call` (`primitives/workers.rs`) sottomettono un
`LispJob` (`primitives/lisp_job.rs`): classe normale, unità il carburante,
nessun argomento finché un'attesa non ne nomina. Un passo riprende la fiber una
volta, o chiama una volta la funzione di `background-call`, e ciò che la fiber
passa a `yield` dice quando viene il passo dopo: `(yield)` dopo l'intervallo
del lavoro, 40 ms se non è dato; `(yield 'continue)` appena tocca di nuovo a
lui; `(yield '(sleep MS))`; `(yield '(wait TOPIC...))`;
`(yield '(wait-at-most MS TOPIC...))`. La tabella completa, con gli esempi, è
nella pagina `background`. Gli argomenti di un'attesa diventano del lavoro, e
restano finché un'attesa dopo non ne nomina altri: `(wait)` e
`(wait-at-most 500)` li tengono. Un altro valore vale l'attesa predefinita,
così un lavoro che passa a `yield` il suo progresso continua a funzionare; una
lista che comincia come una delle forme e non lo è, come `(sleep "soon")`, è un
errore, e il lavoro fallisce con un messaggio che dice la forma.

Ogni passo ha il suo carburante, `worker-fuel` (200 000 per default), letto
quando il lavoro parte, impostato sul thread dell'esecutore all'inizio di ogni
passo e rimesso com'era alla fine. Il carburante è per thread: un comando non
accorcia il passo di un lavoro,
e un lavoro non spende quello di un comando. Un passo che lo esaurisce finisce,
e il lavoro esce come fallito; altrimenti un `(while t)` senza `yield`
fermerebbe l'esecutore, e la colorazione con lui. Durante il passo
`WorkerTurn` segna il thread come «in background» (`in_worker`), e le
primitive che hanno senso solo sul thread dei comandi lo controllano:
`minibuffer-read` rifiuta di aprire un prompt.

`ON-PROGRESS` e `ON-DONE` non girano sull'esecutore: il lavoro le lascia come
callback dovute. `ON-PROGRESS` riceve il nome dopo ogni passo tranne l'ultimo.
`ON-DONE` riceve il nome e `t` per `Exit::Finished`, `nil` per `Failed` e
`Panicked`; dopo `Cancelled` e `Shutdown` non è chiamata, perché chi ha fermato
il lavoro lo sa già.

Le regole, per tutto ciò che un programma Lisp fa in background:

+ *Ciò che può durare più di un frame è un lavoro in background*, avviato da una
  chiamata che dice che cosa fare e che cosa fare alla fine; non una funzione
  che avvia qualcosa e lascia al chiamante di scoprire come è andata.
+ *Un lavoro non disegna.* Cambia lo stato, e il renderer se ne accorge: la
  fine di ogni passo gli dice di guardare.
+ *Un lavoro non suppone che l'editor sia stato fermo.* Il buffer per cui è
  partito può non esserci più, la variabile può essere cambiata, la versione
  letta può essere vecchia di due modifiche: ricontrolla prima di salvare, e ciò
  che non può ricontrollare non lo tiene.
+ *Il progresso è un segnale, non un ridisegno.* Un lavoro dice «c'è
  dell'altro»; che cosa voglia dire lo decide il modo.
+ *Il modo si tiene coerente da sé*, e chiederlo è giusto solo se il segnale
  arriva dove un modo può rispondere senza rischi: per questo le callback girano
  sul thread dei comandi, tra un comando e l'altro (@callback-dovute).
+ *Gli errori vanno nel log, non si sollevano.* Il comando che ha avviato il
  lavoro è finito da tempo, e non c'è nessuno a cui sollevarli. Un lavoro fallito
  dice comunque che è finito, perché un modo che non lo sa aspetta per sempre.
+ *Un lavoro si sostituisce, non si duplica.* Avviarne uno con un nome in uso
  ferma il vecchio, così rivalutare un modulo o premere due volte un tasto non
  fa danni; le callback in coda del vecchio si scartano, e il suo `ON-DONE` non
  viene chiamato, perché chi l'ha sostituito lo sa già.

=== Le callback dovute <callback-dovute>

Una callback chiesta da un lavoro va nella lista `owed` dell'editor
(`owe_callback`), con il nome e il numero del lavoro. Il front-end lo sa da
`next_redraw_in`, che risponde zero finché la lista non è vuota, e la esegue
chiamando `tick`, che chiama `run_owed_callbacks`:

#canvas(height: 5.8cm)[
  #lane(1.6cm, 0cm, 5.7cm, [lavoro (esecutore)], fill: accent.lighten(85%), w: 3.0cm)
  #lane(5.6cm, 0cm, 5.7cm, [`owed`])
  #lane(9.4cm, 0cm, 5.7cm, [ciclo della TUI], fill: accent.lighten(85%))
  #lane(13.3cm, 0cm, 5.7cm, [`run_owed_callbacks`], w: 3.2cm)
  #msg(1.6cm, 5.6cm, 1.2cm, [`owe_callback(nome, id, f, args)`])
  #msg(9.4cm, 5.6cm, 1.8cm, [`next_redraw_in`: `callbacks_owed()`?])
  #msg(5.6cm, 9.4cm, 2.3cm, [sì → attesa zero], dash: "dashed")
  #msg(9.4cm, 13.3cm, 2.9cm, [scade: `tick(&env)`])
  #msg(13.3cm, 5.6cm, 3.4cm, [`take` della lista intera])
  #seq-note(13.0cm, 3.7cm)[`is_latest` falso → scartata]
  #seq-note(10.2cm, 4.45cm)[altrimenti: `begin_command()` e `call_callable`;\ un errore va nel log]
]

La lista si prende intera con un solo `take` prima di eseguire qualunque
callback: una callback che ne chiede altre le aggiunge a una lista nuova, che
girerà al prossimo `tick`, e il lock non resta mai preso mentre gira il Lisp.
Una callback il cui lavoro non parla più per il suo nome (`is_latest`) si
scarta. Le altre girano con un budget di carburante proprio
(`begin_command`), sul thread che possiede l'interfaccia, tra un comando e
l'altro: possono aprire buffer, scrivere messaggi o cambiare finestra come un
comando, ma non sono comandi, e non cambiano `last-command`.

=== Quando ridisegnare

Quasi tutto ciò che è sullo schermo cambia perché l'utente ha fatto qualcosa;
ciò che cambia un lavoro no. Per questo l'editor passa al WorkManager, come hook
di `after_step`, `BackgroundChange::raise`. Alla fine di ogni passo, e quando un
thread dedicato scrive nel suo buffer o finisce (`changed_in_background`), `raise`
chiama una volta la funzione che il front-end ha registrato con
`on_background_change`, e non di nuovo finché il front-end non prende una
fotografia. La TUI la usa per svegliare il suo ciclo.

`next_redraw_in(&env, &frame)` è poi il minimo tra: la scadenza del messaggio
nell'area messaggi (`echo-message-timeout`, 5 s; `nil` non scade mai);
`TURN_INTERVAL` (40 ms) se la colorazione è in corso; il tempo che manca perché
sia passato un `TURN_INTERVAL` dall'ultima fotografia, se un lavoro ha cambiato
qualcosa da allora (`BackgroundChange::due_in`), così un lavoro che fa un passo
al millisecondo non ridisegna il terminale a ogni passo; zero se ci sono
callback dovute. `None` vuol dire che nulla è in sospeso e il front-end può
bloccarsi sull'input. `drag_scroll_in()` aggiunge i 60 ms dello scorrimento
automatico mentre si trascina una selezione oltre il bordo di una finestra.

== Come si prova <prove-sfondo>

#ref-table(
  columns: (3.0cm, 1fr),
  header: ([Livello], [Come]),
  [Il nucleo], [`work_core_tests`: un `Core` guidato a mano su un `ManualClock`, senza thread. Un lavoro che deve durare 30 ms sposta l'orologio dall'interno del suo passo, e ogni test è esatto; un minuto di scheduler simulato richiede microsecondi.],
  [Il WorkManager], [`work_manager_tests`, `work_threads_tests` e `work_watchdog_tests`, con gli aiuti comuni di `work_support`. Quasi tutti usano la modalità manuale; i pochi con thread veri aspettano una condizione con una scadenza, mai un tempo fisso, e chiudono il WorkManager anche quando falliscono.],
  [L'editor e il Lisp], [L'editor dei test nasce in modalità manuale: i lavori girano solo con `ctx.work().run_until_idle(ctx)` o `run_for(ctx, durata)`, e i thread veri solo dove un test li vuole.],
)

Gli altri strumenti:

- *Il watchdog in modalità manuale.* Il thread del test è l'esecutore, quindi un
  test che vuole osservare un passo lo fa eseguire su un altro thread
  (`drive_elsewhere`), sposta l'orologio e chiama `watch_once`.
- *Invarianti su sequenze casuali.* `random_sequences_keep_the_invariants`
  genera sottomissioni, `cancel`, emissioni ed esiti con un generatore scritto
  nel test, a partire da semi fissi, e controlla dopo ogni operazione che ogni
  lavoro stia in un solo posto, che `exit` arrivi una volta sola e che la
  regola di `high-steps` valga.
- *Nessun thread lasciato indietro.* Su Linux,
  `shutdown_leaves_no_thread_behind` conta in `/proc` i thread che portano il
  nome del WorkManager, e dopo `shutdown` non ne trova più.
- *Gli argomenti alla fonte.* `topic_source_tests` conta ciò che ogni fonte
  emette, e segue una modifica fino al lavoro che sveglia.

== Le decisioni <decisioni-sfondo>

+ *Un solo WorkManager per tutto il lavoro asincrono.* Ogni lavoro, a passi o su
  un thread, entra dalla stessa porta, ha un nome e un numero, e si può elencare
  con `work-list` e fermare con `stop-worker`. Scartato: lasciare a ogni
  sottosistema i suoi thread, che nessuno elenca né ferma e che nessuno
  aspetta all'uscita.
+ *Lavori a passi su un esecutore, e un thread dedicato per ciò che si blocca.*
  Un passo non aspetta mai; ciò che deve aspettare ha un thread suo, che il
  WorkManager avvia e conosce. Comporta che un lavoro a passi si scriva come
  una macchina che riprende da dove era rimasta. Scartato: un thread per
  lavoro, che costa uno stack ciascuno e fa girare insieme lavori che
  prendono gli stessi lock.
+ *Lo scheduler misura e segnala, non interrompe.* Un passo lungo finisce nel
  log con il suo costo per unità, e si corregge a mano. Scartati: un budget di
  tempo per passo, che renderebbe il lavoro dipendente dalla macchina e i test
  non deterministici, e l'interruzione di un passo, che Rust non sa fare in
  sicurezza. Il carburante dei lavori Lisp non è un budget di tempo: conta i
  passi del valutatore, uguali su ogni macchina.
+ *Gli argomenti non portano dati.* Un argomento dice «vai a guardare», e ciò
  che serve sta nello stato, dove nasce. Un messaggio con dati va consegnato
  uno per uno, invecchia, e quando una coda si riempie va perso: non può essere
  la fonte della verità. Un contatore costa un incremento, si emette anche
  sotto il lock di un buffer, e di una raffica fa un numero cresciuto.
+ *Il nucleo non ha thread né orologio.* Chi lo guida gli dà i turni, e l'ora
  gli arriva da un `Clock`: così si prova tutto senza thread, e in modo esatto.
  Scartato: un nucleo con il suo thread, che i test potrebbero solo aspettare.
+ *La regola delle classi si conta in passi.* Un passo normale ogni
  `high-steps` passi alti di fila, e non una quota di tempo: la regola è
  deterministica e si prova senza orologio.
+ *Il nucleo scandisce i suoi lavori invece di tenere un heap delle scadenze.*
  I lavori sono decine, non migliaia, e una scansione non può andare fuori
  passo con i lavori come un heap con voci superate. Un heap si aggiunge se le
  misure un giorno lo chiedono.
+ *Una chiamata non tocca il nucleo.* Il registro risponde subito, e il resto
  arriva all'esecutore come messaggio: così `submit` dall'interno di un passo
  non aspetta l'esecutore, che è il thread stesso che chiama. Comporta che
  registro e nucleo tengano due viste dei lavori, e che `jobs()` mostri le
  misure dell'ultima pubblicazione.
+ *Nomi unici, e il nuovo sostituisce il vecchio.* Rivalutare un modulo, o
  premere due volte lo stesso tasto, non duplica un lavoro. Le callback del
  lavoro sostituito si scartano con `is_latest`, che il registro cambia insieme
  al lavoro: due sottomissioni con lo stesso nome, o una sottomissione e uno
  stop, non possono lasciarlo a indicare il lavoro sbagliato.
+ *Un thread non si uccide: gli si chiede di fermarsi.* Rust non ha un modo
  sicuro di uccidere un thread, e uno ucciso mentre tiene il lock di un buffer
  bloccherebbe l'editor al primo accesso. Lo `Stop` e l'azione di sblocco
  coprono ogni caso tranne il thread che non controlla e non si sblocca, che
  viene segnalato e abbandonato all'uscita.
+ *`shutdown` è esplicito.* L'esecutore tiene una copia di `EditorState`, che
  contiene il WorkManager: è un ciclo di riferimenti, e `Drop` non lo rompe.
  Per questo il front-end chiama `shutdown` all'uscita, con una guardia che lo
  fa anche dopo un panic, e lo chiama anche ogni test che avvia dei thread.
+ *Il watchdog scrive il file di log.* Una scrittura che si blocca, per un
  disco pieno o una home di rete, ferma il watchdog e nessun altro, e una
  segnalazione arriva sul file anche quando ciò che segnala è bloccato. Chi
  scrive una riga nel log non scrive mai il file.
+ *Le callback dei lavori girano sul thread dei comandi.* Un lavoro non chiama
  il suo modo: lascia una nota, che il thread dei comandi legge dove leggerebbe
  un tasto. Se un lavoro chiamasse il modo, del Lisp scritto per il thread dei
  comandi girerebbe in un istante qualunque su un altro thread: a metà di un
  inserimento, dentro un hook che scorre una lista. Nessuna cura nella
  callback lo ripara, perché a essere sbagliata non è la callback.
+ *Un comando passa dalla shell.* Ciò che si scrive dopo `M-!` deve voler dire
  ciò che vorrebbe a un prompt, con pipe, redirezioni e glob. Spezzare sugli
  spazi cambierebbe `grep -r "due parole" .`, e riscrivere le regole di quoting
  di una shell darebbe una shell sottilmente diversa da quella che l'utente
  conosce.
+ *Un comando dice nel suo buffer come è finito.* Il thread ha l'`EditorState` e
  non l'ambiente Lisp: non può chiamare una callback quando il comando finisce,
  né mettere un messaggio che scade con un timer che non gira. Scrive nel buffer
  in cui scrive già una riga `--- exited 0 ---`, che è anche la sola traccia che
  resta a chi ha guardato altrove.
+ *Stdout e stderr in una pipe sola.* Arrivano nell'ordine in cui il comando li
  scrive, come a un prompt, e un errore sta accanto all'output di cui parla.
  Letti uno dopo l'altro, un comando che scrive su stderr più di quanto una pipe
  tiene prima di chiudere stdout, una build piena di avvisi, aspetterebbe un
  lettore che aspetta lui.
+ *L'output si legge come byte.* Ciò che arriva si mostra subito, anche una
  barra di avanzamento o una domanda, che non finiscono una riga. Letto come
  testo, un byte Latin-1 in un nome di file chiuderebbe la lettura, e il comando
  morirebbe della pipe chiusa sotto di lui.
+ *Un comando ha un gruppo di processi suo, e niente da leggere.* Fermarlo deve
  raggiungere ciò che ha avviato, non solo la shell: i figli terrebbero aperta
  la pipe e sopravviverebbero all'editor. Un gruppo suo è anche un gruppo a cui
  il terminale non appartiene: un comando che apre il terminale, come una
  richiesta di password, viene fermato dal sistema invece di prendere i tasti
  dell'editor, e il `CONT` dopo il `TERM` fa sì che lo senta.

// ===========================================================================
= Lo schermo <frame>
// ===========================================================================

== Finestre

Le finestre affiancate formano un albero (`LayoutNode`): una foglia è una
`Window` (id, nome del buffer, scorrimento orizzontale e verticale, punto
proprio quando non ha il focus, larghezza del margine), un nodo interno è una
divisione orizzontale o verticale con una `Division` (`Ratio`, `FirstFixed`,
`SecondFixed`). Sopra l'albero stanno le finestre #emph[flottanti]
(`FloatingWindow`: una finestra più un rettangolo, un bordo facoltativo, un
titolo e la finestra a cui restituire il focus quando si chiude): il
minibuffer, il `*Backtrace*`, i popup dei moduli.

#canvas(height: 4.8cm)[
  #node(4.0cm, 0.5cm, 4.2cm, 0.7cm, [`Split` verticale `Ratio(0.5)`], fill: paper)
  #carrow((3.0cm, 0.85cm), (1.8cm, 1.6cm))
  #carrow((5.0cm, 0.85cm), (6.0cm, 1.6cm))
  #node(1.8cm, 1.95cm, 2.6cm, 0.7cm, [`Leaf` 0: `main.rs`])
  #node(6.0cm, 1.95cm, 3.4cm, 0.7cm, [`Split` orizzontale], fill: paper)
  #carrow((5.2cm, 2.3cm), (4.6cm, 3.05cm))
  #carrow((6.8cm, 2.3cm), (7.4cm, 3.05cm))
  #node(4.4cm, 3.4cm, 2.6cm, 0.7cm, [`Leaf` 1: `lib.rs`])
  #node(7.6cm, 3.4cm, 2.8cm, 0.7cm, [`Leaf` 2: `*Occur*`])
  #rect(width: 0cm, height: 0cm)
  #place(dx: 10.2cm, dy: 0.3cm, rect(width: 5.6cm, height: 3.7cm, stroke: 0.6pt + dim))
  #place(dx: 12.9cm, dy: 0.3cm, line(start: (0cm, 0cm), end: (0cm, 3.7cm), stroke: 0.6pt + dim))
  #place(dx: 12.9cm, dy: 2.1cm, line(start: (0cm, 0cm), end: (2.9cm, 0cm), stroke: 0.6pt + dim))
  #note(10.5cm, 1.6cm)[0]
  #note(14.2cm, 0.9cm)[1]
  #note(14.2cm, 2.8cm)[2]
  #place(dx: 11.5cm, dy: 1.4cm, rect(width: 3.0cm, height: 0.9cm, fill: white, stroke: 0.8pt + accent))
  #note(11.7cm, 1.65cm, colour: accent)[flottante (minibuffer)]
  #note(0.2cm, 4.3cm)[Le foglie si dividono lo spazio; le flottanti sono disegnate sopra, con il loro rettangolo.]
]

Il compartimento `Windows` tiene l'albero, le flottanti, l'id con il focus, il
prossimo id, lo stato di un trascinamento del mouse e l'ultimo clic (per il
doppio clic entro 400 ms). Le operazioni che tolgono una finestra rispondono con
un `WindowRemoved` (`No`, `Yes`, `Refocus(id)`) che dice alla facciata se e
verso quale finestra spostare il focus.

== `snapshot`: dalla memoria alla fotografia <snapshot>

`snapshot(&env, larghezza, altezza)` costruisce il `FrameSnapshot`. Prende i
lock in un ordine fisso e non fa I/O; il disegno avviene dopo, senza lock.

+ Abbassa il segnale del lavoro di sfondo (`BackgroundChange::taken`): ciò che
  i lavori hanno cambiato fin qui è in questa fotografia.
+ Legge dalle variabili Lisp e dai campi piccoli tutto ciò che serve
  #emph[prima] di toccare finestre e buffer: `echo-message-timeout`, il tema,
  l'input in sospeso (`pending_input`), il messaggio della keymap transitoria,
  `mode-line-format`, `window-separator`, la configurazione del margine
  (`display-line-numbers`), se la colorazione è in corso.
+ Legge l'area messaggi, vuota se il messaggio è scaduto.
+ Con `windows` in scrittura (lo scorrimento può dover seguire il punto) e
  `buffers` in lettura, attraverso `windows_mut_with_buffers`, compone ogni
  foglia dell'albero nel rettangolo del frame meno l'ultima riga, raccogliendo
  i separatori; poi compone le flottanti.
+ Consuma il testo destinato agli appunti (`take_pending_clipboard`).

Per ogni foglia la composizione fa: scorrimento per mantenere visibile il punto;
layout delle righe visibili, compreso il testo virtuale; margine dei numeri di
riga (assoluti o relativi, larghezza minima `display-line-numbers-width`, 3 per
default, massimo 20); evidenziazioni nell'ordine #emph[sintassi → overlay →
regione → testo virtuale], perché le ultime vincono; posizione del cursore
relativa alla finestra; riga di stato. Le finestre flottanti non hanno margine
né riga di stato, e ricevono solo regione e testo virtuale #rapporto("TUI-6").

#ref-table(
  columns: (4.4cm, 1fr),
  header: ([Campo di `FrameSnapshot`], [Contenuto]),
  [`views`], [Un `RenderableWindowView` per finestra: rettangolo, nome del buffer, titolo, focus, cursore relativo, righe di testo, evidenziazioni (riga, colonne, faccia), riga di stato, bordo, margine e sua larghezza.],
  [`separators`], [Rettangoli e carattere delle linee tra finestre affiancate.],
  [`echo_message`, `prompt`], [Il messaggio e l'eventuale offerta della keymap transitoria, che lo sostituisce.],
  [`pending_input`], [`C-u 4 C-x-`: ciò che l'editor sta leggendo.],
  [`theme`], [Gli stili delle facce (`Arc<Theme>`).],
  [`focused_window_id`, `width`, `height`], [Come dice il nome.],
  [`colouring_pending`], [La colorazione non ha finito: ridisegnare tra 40 ms.],
  [`clipboard`], [Testo che il front-end deve mettere negli appunti di sistema, se presente.],
)

La riga di stato segue `mode-line-format` (predefinito
`" %* %b   %m %s  L%l C%c   %p "`): `%b` nome del buffer, `%f` file (o nome),
`%m` modo, `%s` lo stato che il modo tiene nel buffer (`mode-line-state`, come
il `NORMAL` di vim), `%l` riga (da 1), `%c` colonna (da 0), `%p` posizione
(`All`, `Top`, `Bot` o percentuale), `%*` `**` se modificato e `--`
altrimenti, `%%`.

== Il renderer della TUI

`render_to` disegna una fotografia in quest'ordine: nasconde il cursore e pulisce
lo schermo; disegna i separatori (prima delle finestre, perché una flottante
deve poterli coprire); per ogni vista disegna bordo e titolo, ogni riga del
rettangolo riempita fino alla larghezza (così una flottante è opaca), il
margine cella per cella, le evidenziazioni sopra il testo, la riga di stato
sotto il rettangolo; poi l'area messaggi nell'ultima riga, l'input in sospeso
allineato a destra nella stessa riga, il cursore #emph[per ultimo] e infine,
se c'è, la sequenza OSC 52 per gli appunti.

I colori sono risolti dal front-end secondo la profondità del terminale:
24 bit con `COLORTERM` che contiene `truecolor` o `24bit`; il cubo a 256 colori
(o la scala di grigi, se più vicina) con `TERM` che contiene `256color`;
altrimenti la voce più vicina dei 16 colori della tavolozza, nell'ordine di
`NAMED_COLORS`.

Gli appunti in uscita passano da OSC 52 perché l'editor non sa di avere un
terminale e un programma esterno (`xclip`, `pbcopy`) non funzionerebbe via SSH.
In entrata non c'è nulla da leggere: il terminale consegna gli appunti con
l'incolla «bracketed», che diventa `insert-pasted-text`.

== Il mouse <mouse>

Con `mouse-mode` attivo la TUI chiede al terminale di riportare il mouse.
`handle_mouse_event` traduce l'evento in un `Hit` (testo di una finestra, riga
di stato, separatore, flottante, niente), e da lì in un comando: clic per
spostare il punto e il focus, doppio clic, trascinamento per selezionare (con
scorrimento automatico ogni 60 ms oltre il bordo), trascinamento di un
separatore o di una riga di stato per ridimensionare, rotella per scorrere la
finestra sotto il puntatore. Risponde `true` solo se ha cambiato qualcosa. Le
domande sulle finestre --- che cosa c'è sotto il puntatore, il trascinamento in
corso, l'ultimo clic --- le fa a `windows.rs`; il punto di un clic diventa una
colonna del buffer in `buffer_column`, che compone la sola riga cliccata,
perché il testo virtuale fa di una colonna sullo schermo un'altra colonna nel
buffer. I comandi sono in `primitives/mouse.rs`, e si registrano senza
specifiche degli argomenti: i loro argomenti li passa `handle_mouse_event`, non
un prompt.

== Le decisioni <decisioni-frame>

+ *Una fotografia sola, presa tutta insieme.* I lavori, i thread dei comandi
  di shell e quelli di `spawn` cambiano lo stato mentre il front-end disegna:
  un renderer che leggesse sei lock in sei istanti comporrebbe un frame mai
  esistito. `snapshot` legge tutto sotto i lock, senza I/O, e il disegno viene
  dopo, senza lock.
+ *`snapshot` prende le finestre in scrittura.* Comporre una finestra ne sposta
  lo scorrimento per tenere visibile il punto: disegnare cambia lo stato. Va
  bene finché disegna un thread solo; per comporre più frame in parallelo,
  quel raccordo dovrebbe diventare un passo dopo ogni comando.
+ *L'editor dice al front-end quando ridisegnare.* Quasi tutto cambia per un
  evento, ma un messaggio scade, la colorazione arriva, un lavoro riempie un
  buffer: il front-end chiede `next_redraw_in` e riceve una durata sola, e un
  secondo front-end ha lo stesso comportamento chiedendo la stessa cosa.
  `None` è la risposta comune, e un editor fermo non costa nulla.
+ *Le flottanti non hanno margine né riga di stato.* Una flottante è un prompt
  o una striscia di scelte, poche righe lette e chiuse: numerarle non dice
  nulla, e il bordo dice già che cosa sono.
+ *Il mouse calcola dove, un comando decide che cosa.* Dove cade un clic
  dipende dal layout e dallo scorrimento di ogni finestra, e lo calcola
  l'editor; che cosa significa passa dai comandi come un tasto: un passo di
  undo, un `post-command-hook`, un nome che `M-x` e il log conoscono, un
  legame che il Lisp può cambiare.
+ *Un clic chiude la selezione.* Una regione va dal mark al punto, e spostare
  il punto ne sposta un capo: senza questo, ogni clic con una selezione aperta
  ne trascinerebbe il capo dove si è cliccato, anche lontano, e lascerebbe una
  selezione su mezzo file che nessuno ha chiesto. Il mark si disattiva e non si
  cancella: `exchange-point-and-mark` ci torna.
+ *Un trascinamento continua nella finestra in cui è cominciato.* Il seguito e
  la fine di un trascinamento si risolvono prima di chiedere che cosa c'è sotto
  il puntatore: dopo poche righe il puntatore è spesso su un'altra finestra o
  fuori dal frame, e una selezione che cambiasse buffer a metà non sarebbe una
  selezione.
+ *`mouse-mode` vale per ogni front-end.* `handle_mouse_event` lo controlla da
  sé, non solo dove il terminale accende la cattura: una finestra grafica, che
  riceve sempre il mouse, gli ubbidisce allo stesso modo.

// ===========================================================================
= Macro di tastiera <macro>
// ===========================================================================

Una macro registra #emph[tasti], non comandi: `C-u 5`, un tasto letto da
`read-key-sequence` e perfino un tasto non associato fanno parte di ciò che
l'utente ha premuto. `handle_key_event` chiama `record_keys` nei punti in
cui un tasto viene consumato. La riproduzione (`replay_keys`) rimanda i tasti a
`handle_key_event`, con un `OneGroup` per ripetizione, e si ferma al primo
comando fallito, confrontando `command_errors` prima e dopo ogni tasto. Una
macro che richiama macro può annidarsi fino a 16 livelli (`MAX_DEPTH`).

Le primitive stanno in `primitives/macros.rs`. `define-kbd-macro` fa di una
stringa di tasti un comando con un nome: una funzione Lisp il cui corpo chiama
`kmacro-call-keys` con la stringa, e la cui docstring dice quali tasti preme.
Una stringa che non si legge come sequenza di tasti è rifiutata subito. Il
contatore (`kmacro-insert-counter`, `kmacro-set-counter`, `kmacro-counter`)
sta nel compartimento accanto ai tasti; la larghezza di `kmacro-insert-counter`
non ha limite #rapporto("CMD-3").

== Le decisioni <decisioni-macro>

Tre decisioni sulla macro stanno con il suo compartimento (@decisioni-stato):
una macro registra tasti, registrazione e riproduzione stanno insieme, le
riproduzioni annidate hanno un tetto. Le altre sono della facciata, e
l'ultima delle primitive.

+ *La riproduzione passa dalla porta dei tasti.* Ogni tasto registrato torna a
  `handle_key_event`, la stessa porta di un tasto del terminale: un secondo
  dispatcher sarebbe un secondo insieme di regole su argomenti prefisso, keymap
  transitorie, catture e prompt, tenuto in passo a mano. Il prezzo è il
  rientro, ed è già pagato: il carburante conta la profondità e solo lo scope
  più esterno ricarica, così i comandi di una riproduzione spendono il budget
  della riproduzione.
+ *Una riproduzione si ferma al primo comando fallito.* Ogni tasto dopo un
  errore sarebbe premuto in uno stato che la registrazione non ha mai visto.
  `run_command_form` non restituisce l'errore --- un tasto non ha un chiamante
  a cui restituirlo ---: `report_error` incrementa `command_errors`, che la
  riproduzione confronta prima e dopo ogni tasto. Un conteggio, che nessuno
  deve azzerare.
+ *Nessun tasto interrompe una riproduzione.* Il ciclo non legge tasti: è
  dentro un comando, e il thread che consegnerebbe `C-g` è quello che lo
  esegue. Una macro che richiama sé stessa la ferma il tetto di profondità,
  non il budget: ogni livello è una vera chiamata attraverso il gestore dei
  tasti, e lo stack finirebbe molto prima, interrompendo il processo. Per lo
  stesso motivo una riproduzione disegna un frame solo, quando il comando
  restituisce.
+ *Un gruppo di undo per ripetizione.* Ogni ripetizione ha il suo timbro e il
  suo `OneGroup`: venti ripetizioni sono venti passi, e una macro andata storta
  alla diciannovesima si riporta indietro fino lì.
+ *Una macro con un nome è una funzione Lisp.* Un comando qui è una funzione
  Lisp, e una macro costruita come chiusura Rust sarebbe invisibile a
  `describe-function` e a tutto ciò che legge una definizione; la sua docstring
  dice i tasti, che sono tutto ciò che c'è da sapere di lei. Una stringa
  sbagliata si rifiuta quando si definisce la macro, perché quando la si esegue
  l'unica cosa da dire sarebbe che una macro che nessuno vede è vuota.

// ===========================================================================
= Le primitive dell'editor <primitive>
// ===========================================================================

== Il registro

Ogni modulo di `primitives/` ha una funzione `install(into: &Registry)` che
registra le sue primitive con `into.function(nome, puntatore, DOC)` o, per i
comandi, `into.command(nome, puntatore, specifiche, DOC)`.
`install_primitives` chiama tutti i moduli. La macro `primitive!` genera la
firma richiesta da `LispPrimitive`; `args.rs` contiene i lettori degli argomenti
(@lettori-argomenti).

Ogni docstring comincia con la firma, nella forma `(nome ARG):` seguita da una
frase: è ciò che mostrano i comandi di aiuto. `doc_faithfulness_tests`
confronta ciò che l'aiuto racconta con l'editor reale; `primitive_shape_tests`
verifica le forme condivise dalle primitive, come il controllo dell'arità.

#ref-table(
  columns: (2.8cm, 0.8cm, 1fr),
  header: ([Modulo], [N.], [Contenuto]),
  [`movement`], [20], [Movimenti per carattere, riga, parola e paragrafo, ai bordi della riga e del buffer, `goto-line`; il punto come numero (`point`, `goto-char`, `current-column`).],
  [`buffers`], [18], [Creare, cambiare, chiudere buffer; contenuto; sola lettura; dati appesi (`buffer-put`, `buffer-get`).],
  [`modes`], [16], [`make-mode`, `add-hook`, regole e regioni di sintassi, `add-auto-mode`, tabella sintattica, `syntax-ppss`.],
  [`windows`], [15], [Dividere, chiudere, scegliere, scorrere finestre; `recenter`, `display-buffer-at-bottom`, finestre flottanti.],
  [`killing`], [14], [Le kill, `kill-ring-save`, `yank`, `yank-pop`, `kill-new` e il resto del kill ring.],
  [`verbs`], [14], [Maiuscole, spazi, righe vuote, unione e duplicazione di righe, trasposizioni, `zap-to-char`.],
  [`general`], [13], [`eval-file`, `define-key`, `define-repeat-key`, `repeat`, `log`, `backtrace`, `regexp-opt`, keymap transitorie.],
  [`results`], [13], [Liste di posizioni (`occur`, compilazione) e `next-error`.],
  [`macros`], [12], [Registrazione e riproduzione delle macro, contatore.],
  [`minibuffer`], [12], [`minibuffer-read`, conferma, annullamento, completamento, storia.],
  [`replace`], [12], [Sostituzione e `query-replace`.],
  [`commands`], [10], [`call-interactively`, `M-x`, `register-command` e il registro dei comandi.],
  [`isearch`], [10], [Ricerca incrementale.],
  [`rectangle`], [10], [Operazioni sui rettangoli.],
  [`region`], [9], [Mark e regione, `keyboard-quit`.],
  [`workers`], [9], [`define-worker`, `background-call`, `stop-worker`, `running-workers`, `emit-event`, le opzioni dello scheduler, `work-list`.],
  [`completion`], [8], [`completion-at-point`, le sue fonti, `fuzzy-filter`.],
  [`files`], [8], [Aprire, salvare, rileggere e recuperare un file; `write-file`.],
  [`edits`], [7], [Le due porte; `self-insert`, `insert`, l'incolla, `insert-newline`, `delete-region`, `delete-char`, `delete-backward-char`.],
  [`file_names`], [7], [Percorsi resi assoluti e smontati, `list-dir`, `match-list`, il completamento dei nomi dei file.],
  [`filesystem`], [7], [Esistenza, cancellazione e rinomina di file, `directory-files-recursive`, `read-file-to-string`.],
  [`help`], [7], [Ciò che serve all'aiuto: `key-binding`, `where-is`, `keymap-bindings`, `read-key-sequence`. I comandi `describe-*` sono in `help.lisp`.],
  [`match_data`], [7], [`string-match` e i dati della corrispondenza.],
  [`sexp`], [7], [Movimenti e kill per espressioni bilanciate, `up-list`, `down-list`.],
  [`comments`], [6], [Commentare e decommentare.],
  [`mouse`], [6], [Clic, trascinamenti e rotella.],
  [`theme`], [6], [Facce e stili.],
  [`overlays`], [5], [Overlay.],
  [`quit`], [5], [`quit`, `save-some-buffers` e le loro risposte.],
  [`undo`], [5], [`undo`, `redo`, `undo-boundary`, `undo-join`, `set-undo-limit`.],
  [`ask`], [4], [Domande sì/no con keymap transitoria.],
  [`environment`], [4], [`getenv`, `setenv`, `path-separator`, `data-directory`.],
  [`shell`], [4], [Comandi di shell, in background o fino alla fine; `parse-overstrike`.],
  [`virtual_text`], [4], [Testo virtuale.],
  [`indentation`], [3], [`current-indentation`, `previous-indentation`, `indent-line-to`.],
  [`scan`], [1], [`scan-buffer`: tutte le corrispondenze di un modello in un buffer, come voci di una lista di risultati.],
)

Il totale è 318, a cui si aggiungono le 86 primitive di base dell'interprete.
L'elenco completo dei nomi è nell'appendice (@elenco-primitive).

== Le primitive di modifica <primitive-modifica>

Le primitive che cambiano il testo o spostano il punto stanno in tredici file,
uno per concetto. Tutte cambiano il testo soltanto dalle due porte
(@due-porte), e nessuna prende il lock di un altro compartimento dentro la
closure di un buffer: chiede prima ciò che le serve, lavora nella closure e
riferisce dopo (@ordine-lock).

#ref-table(
  columns: (3.1cm, 1fr),
  header: ([File], [Contenuto]),
  [`edits.rs`], [Le due porte (`insert_text`, `delete_range`), `changed`, `edited`; le primitive che inseriscono e cancellano caratteri: `self-insert`, `insert`, `insert-pasted-text`, `insert-newline`, `delete-region`, `delete-char`, `delete-backward-char`.],
  [`undo.rs`], [`undo`, `redo`, `undo-boundary`, `undo-join`, `set-undo-limit` (@undo).],
  [`movement.rs`], [Il movimento per carattere, riga, parola e paragrafo, ai bordi della riga e del buffer, `goto-line`; il punto come numero; le funzioni che dicono dove finiscono una parola, una riga, un paragrafo.],
  [`killing.rs`], [Le kill, da `kill-line` a `kill-region`; `kill-ring-save`, `yank`, `yank-pop`, `kill-new` e il resto dell'anello; `cut_out`.],
  [`sexp.rs`], [Il movimento per espressioni bilanciate, dentro e fuori dalle liste, e le due kill di espressioni.],
  [`indentation.rs`], [`current-indentation`, `previous-indentation`, `indent-line-to`.],
  [`verbs.rs`], [Maiuscole, spazi, righe, trasposizioni, `zap-to-char`.],
  [`region.rs`], [Il mark e la regione, `keyboard-quit`.],
  [`rectangle.rs`], [I comandi sui rettangoli, la cui geometria è in `text/rectangle.rs` (@rettangoli).],
  [`comments.rs`], [Commentare e decommentare.],
  [`macros.rs`], [Registrare e riprodurre i tasti, e il contatore (@macro).],
  [`match_data.rs`], [`string-match` e i dati della corrispondenza.],
  [`scan.rs`], [`scan-buffer`.],
)

I conteggi si leggono con i lettori di `args.rs`: `movement_count`, con un
segno, per i movimenti, dove un conteggio negativo va all'indietro;
`repeat_count` per le cancellazioni, dove un conteggio negativo diventa 0
#rapporto("CMD-6"). Né l'uno né l'altro ha un limite #rapporto("CMD-3").

Le due porte restituiscono `false` in un buffer in sola lettura, e chi le
chiama lo deve dire: `edited(ctx, happened)` scrive «Buffer is read-only»
nell'area messaggi e risponde nil, chiamata dopo la closure, perché l'area
messaggi è un compartimento a sé. La usano le primitive di `edits.rs`,
`kill-region`, `yank`, `yank-pop` e `indent-line-to`. Le altre kill, i verbi, i
commenti e i rettangoli tacciono; `kill-rectangle` conserva un blocco che non
ha tolto, e `kmacro-insert-counter` segnala un errore con altre parole
#rapporto("BUF-4").

=== Muovere il punto <movimento>

`movement.rs` ha due parti. Le funzioni sul testo conoscono solo
`BufferTrait`: `goto_offset`, `line_length`, `line_bounds`, `is_blank_line`,
`is_word_char`, `word_forward`, `word_backward`, `paragraph_forward` e
`paragraph_backward`. Le usano le primitive del file, e con loro le kill, i
verbi, i commenti, i rettangoli, il mouse e le liste di risultati: c'è un posto
solo che dice dove finisce una parola. Una parola è una sequenza di lettere,
cifre e trattini bassi, senza una tabella sintattica da interrogare; un
paragrafo finisce alla riga vuota che lo segue.

`forward-char` e `backward-char` si muovono per offset, quindi attraversano un
fine riga come `C-f`. `next-line` e `previous-line` tengono una #emph[colonna
obiettivo]:

#estratto("primitives/movement.rs")[
```rust
// Chiesti prima del lock del buffer: sono altri compartimenti.
let continuing = ctx.last_command_is("next-line") || ctx.last_command_is("previous-line");
let remembered = ctx.goal_column();
let goal = ctx.with_current_buffer_mut(|buf| {
    let (line, col) = buf.text.cursor_pos();
    let goal = match (continuing, remembered) {
        (true, Some(goal)) => goal,      // in una serie: la colonna di partenza
        _ => col,                        // altrimenti quella del punto
    };
    // ... la riga di arrivo, limitata al buffer
    buf.text.cursor_move(target, goal);  // limita la colonna alla riga
    goal
});
ctx.set_goal_column(Some(goal));
```
]

Il punto come numero --- `point`, `point-min`, `point-max`, `goto-char`,
`line-number-at-point`, `current-line`, `current-column` --- serve al codice
che ha calcolato una posizione invece di descrivere un movimento: un risultato
di ricerca, un posto salvato. Conta in caratteri, come il mark, la regione e la
storia dell'undo, e una colonna è un carattere, anche un tab.

=== Uccidere e incollare <uccidere>

Ogni kill toglie il testo che il movimento con lo stesso nome attraversa,
chiedendo alla stessa funzione dove finisce: `kill-word` chiede `word_forward`,
`kill-paragraph` `paragraph_forward`, `kill-sexp` lo scanner. Una kill mette
nell'anello ciò che toglie, una cancellazione (`delete-char`,
`delete-backward-char`, `delete-region`) lo scarta, come in Emacs.

Le kill di testo passano da `cut_out`, che cancella con `delete_range` e
restituisce ciò che ha tolto, o niente se il buffer ha rifiutato. L'anello lo
riceve dopo la closure del buffer:

#estratto("primitives/killing.rs")[
```rust
primitive!(kill_word, args, env, ctx, {
    let count = repeat_count(args)?;          // fuori: un `?` nella closure
    let killed = ctx.with_current_buffer_mut(|buf| {
        let from = buf.text.cursor_pos_1d();
        let to = word_forward(&buf.text, from, count);
        cut_out(buf, from, to)                // il testo tolto, o ""
    });
    ctx.kill(killed, Direction::Forward, &env);  // il lock del buffer è reso
    Ok(ELispExp::nil())
});
```
]

`kill-region` e `kill-ring-save` prendono la regione con `require_region`, di
`region.rs`. `kill-ring-save` disattiva il mark: copiare non è una modifica, e
nient'altro lo farebbe. `yank-pop` sostituisce il testo appena incollato con la
voce precedente, togliendo e rimettendo attraverso le porte. Se accodare a una
voce o aggiungerne una nuova lo decide il compartimento `KillYank`, e il
rettangolo ucciso sta in un campo suo (@kill-ring).

=== Espressioni e indentazione <espressioni-indentazione>

I comandi sulle espressioni --- `forward-sexp`, `backward-sexp`, `kill-sexp`,
`backward-kill-sexp`, `up-list`, `backward-up-list`, `down-list` --- leggono
la tabella sintattica del modo prima del lock del buffer, perché
`current_syntax_table` legge il buffer per sapere il suo modo. Poi chiedono
allo scanner di `modes/sexp.rs` (@modi), che riparte dal punto di ripresa più
vicino della cache di scansione (`scan_resume`) invece che dall'inizio del
buffer. Una parola si riconosce guardando i caratteri, un'espressione
lessicando il testo con la tabella: una `)` in una stringa o in un commento è
testo, e lo stesso testo si muove in modo diverso in due modi.

Le primitive dell'indentazione non decidono nulla. La colonna giusta per una
riga è una domanda sulla lingua, e la fa `indent.lisp`: `indent-line` cerca un
`indent-function` sul simbolo del modo e, se non c'è, allinea alla riga sopra
con `previous-indentation`. In Rust c'è ciò che Lisp non potrebbe fare in modo
sicuro o veloce: leggere l'indentazione di una riga (`current-indentation`,
spazi e tab) e riscriverla attraverso le porte, in un passo di undo
(`indent-line-to`, che scrive solo spazi). `indent-line-to` risponde nil se la
riga era già così, ed è ciò che `indent-for-tab-command` legge per scegliere
il suo terzo caso.

=== I verbi <verbi>

I verbi --- `upcase-word`, `capitalize-word`, `just-one-space`,
`delete-blank-lines`, `join-line`, `transpose-chars`, `zap-to-char` e gli altri
--- sono poche lettere di chirurgia, e trovano le parole con le funzioni di
`movement.rs`: `M-t` traspone esattamente ciò che `M-f` attraversa. Ogni verbo
dice nella sua docstring dove lascia il punto. I verbi delle maiuscole, con una
regione, lavorano sulla regione e lasciano il punto dov'è; senza, lavorano
sulla parola dopo il punto e lo lasciano dopo di essa. `join-line` chiede alla
tabella sintattica se un carattere apre o chiude, per non mettere uno spazio
dopo un'apertura o prima di una chiusura.

Quasi tutti passano da `replace_span`, che sostituisce un tratto e mette il
punto dove il verbo vuole. `zap-to-char` legge il suo carattere con una cattura
di tasti (@cattura), armata con il nome di `zap-to-char--do`: la cattura cita
ciò che riceve prima di chiamarlo, e un simbolo si cerca tra le funzioni.

=== Mark e regione <regione>

`set-mark` mette il mark e lo attiva; con INCLUSIVE i caratteri alle due
estremità sono nella regione, come in una selezione modale. `region-beginning`,
`region-end`, `kill-region` e `kill-ring-save` senza una regione attiva danno
un errore, «The mark is not set now, so there is no region».
`exchange-point-and-mark` funziona anche con un mark non più attivo, e limita
la sua posizione alla lunghezza del testo: un mark inattivo non segue le
modifiche, e può indicare oltre la fine. `keyboard-quit` abbandona una
sequenza di tasti o un argomento prefisso a metà e disattiva il mark; non
ferma un comando che gira.

=== Più modifiche in un comando <modifiche-multiple>

I comandi dei commenti e dei rettangoli fanno più modifiche in un comando, e
ogni modifica sposta gli offset di tutto ciò che viene dopo. Le due famiglie
fanno la stessa cosa: calcolano le modifiche una volta, sul testo com'è, e le
applicano dall'ultima alla prima, quando gli offset di ciascuna non sono stati
spostati da nessuna di quelle ancora da fare. Commentare tre righe applicando
dall'alto le aperture calcolate all'inizio le mette tutte sulla prima:

#block(breakable: false)[#ref-table(
  columns: (1.4cm, 1fr, 1fr),
  header: ([Passo], [Dall'alto], [Dal basso]),
  [---], [`ab\ncd\nef`, aperture a 0, 3, 6], [lo stesso],
  [1], [a 0: `// ab\ncd\nef`], [a 6: `ab\ncd\n// ef`],
  [2], [a 3: `// // ab\ncd\nef`], [a 3: `ab\n// cd\n// ef`],
  [3], [a 6: `// // // ab\ncd\nef`], [a 0: `// ab\n// cd\n// ef`],
)]

I commenti mettono le loro modifiche in un elenco ordinato, che `apply`
applica al contrario; poi rimette il punto dove l'utente stava, calcolandolo
dallo stesso elenco, perché il buffer non sa che i caratteri cresciuti davanti
al punto vanno contati. La grafia viene dal modo (`set-comment-syntax`), che la
colorazione, `forward-sexp` e lo scanner leggono dallo stesso posto: un modo
che non ha detto nulla riceve un messaggio, e un linguaggio con soli commenti
a blocco riceve le righe avvolte in uno. Decommentare toglie l'apertura e lo
spazio che il comando aveva messo, non di più: `//   x` tiene l'indentazione
che era dentro il commento.

I rettangoli calcolano gli span una volta (`spans`, @rettangoli) e
`each_line` li percorre dal basso. Inserire allunga prima una riga corta fino
alla colonna (`insert_at_edge`); `clear-rectangle` non allunga nulla;
`yank-rectangle` aggiunge in fondo al buffer le righe che mancano al blocco, e
su una riga dove il blocco ha una stringa vuota non scrive spazi. Ogni comando
che cambia il buffer lascia il punto all'angolo in alto a sinistra;
`copy-rectangle-as-kill` lo lascia dov'è. In un buffer in sola lettura i
rettangoli ignorano il rifiuto #rapporto("BUF-4").

=== I dati della corrispondenza <dati-corrispondenza>

`string-match` ha la forma di Emacs: risponde dove comincia la corrispondenza
e lascia i gruppi nei #emph[dati della corrispondenza], che `match-beginning`,
`match-end`, `match-string` e `match-data` leggono dopo, e `set-match-data`
riscrive. Il prezzo è uno stato che sopravvive alla chiamata: una funzione
chiamata tra la corrispondenza e la lettura può sovrascriverlo, e la risposta è
quella di Emacs, `save-match-data` (in `commands.lisp`). `string-match-p`
non scrive nulla.

I dati stanno in una variabile del thread, `MATCH_DATA`: un passo di un lavoro
Lisp, sul thread dell'esecutore (@sfondo), non cambia ciò che un comando sta
per leggere, mentre i lavori sullo stesso thread li condividono, e lì la
risposta è di nuovo `save-match-data`. Le posizioni sono caratteri, come per
`substring` e `length`: il motore dà byte, e `search` li converte. È diversa da
Emacs la sintassi delle espressioni: i gruppi si scrivono `(...)`, non
`\(...\)`, perché il motore è quello della ricerca (@ricerca), lineare perché
senza lookahead e senza backreference.

=== `scan-buffer` <scan-buffer>

`scan-buffer` dà a Lisp `Pattern::scan` (@ricerca): tutte le corrispondenze in
un passaggio, per occur, grep o un linter che legge il proprio output. Risponde
`(TRUNCATED ENTRIES)`, con le voci nella forma delle liste di risultati
(@risultati), `(KIND SOURCE LINE COLUMN OFFSET TEXT MATCH-START MATCH-END)`,
costruite dalle stesse due funzioni, `Entry::new` ed `entry_form`. Si ferma a
`LIMIT` corrispondenze, 10 000 se nessuno lo dice, e allora TRUNCATED è `t`.

== File, buffer, finestre e domande <primitive-interfaccia>

Le altre primitive stanno anch'esse in un file per concetto. I comandi di shell
e i lavori Lisp sono nel capitolo sul lavoro in background (@comandi-shell,
@lavori-lisp).

#ref-table(
  columns: (3.1cm, 1fr),
  header: ([File], [Contenuto]),
  [`files.rs`], [Visitare un file e riscriverlo: `find-file`, `save-buffer`, `write-file`, `revert-buffer`, `recover-file`, con le metà che le loro domande chiamano una volta risposte; `write_buffer` (@file-su-disco).],
  [`file_names.rs`], [I nomi dei file: `expand_path`, che rende assoluto un percorso, le funzioni che lo smontano, l'elenco di una directory, i candidati del completamento.],
  [`filesystem.rs`], [Il disco: `file-exists-p`, `delete-file`, `rename-file`, `directory-files-recursive` con `walk_files`, `read-file-to-string`.],
  [`environment.rs`], [`getenv`, `setenv`, `path-separator`, `data-directory`.],
  [`quit.rs`], [`quit` e `save-some-buffers`, e `has_unsaved_work`, che dice se chiudere un buffer perderebbe qualcosa.],
  [`buffers.rs`], [Creare, cambiare, leggere e chiudere buffer; i valori Lisp appesi a un buffer; ciò che un buffer dice di sé.],
  [`windows.rs`], [Dividere, chiudere, scegliere e scorrere le finestre, `recenter`, le finestre flottanti.],
  [`mouse.rs`], [I comandi di un clic, di un trascinamento e della rotella (@mouse).],
  [`ask.rs`], [Le domande: `y-or-n`, `yes-or-no`, e `ask_with_keys` per chi ne fa di sue.],
  [`args.rs`], [I lettori degli argomenti, che usano tutti gli altri file.],
  [`shell.rs`], [I comandi di shell (@comandi-shell).],
  [`workers.rs`, `lisp_job.rs`], [I lavori Lisp e le opzioni dello scheduler (@lavori-lisp).],
)

=== I nomi dei file <nomi-file>

`expand_path` fa di ciò che l'utente scrive un percorso assoluto: espande `~`
con `HOME`, o con `USERPROFILE` se `HOME` manca, lo risolve rispetto alla
directory di lavoro se è relativo, e toglie `.` e `..` per nome, senza chiedere
al disco. `expand_path_in` fa lo stesso rispetto a un'altra directory: è il
secondo argomento di `expand-file-name`, per un nome scritto in un elenco di
file. Ne passano `find-file`, `write-file`, le primitive del disco, la ricerca
nei file e il salvataggio automatico.

Le altre funzioni smontano un nome senza toccare il disco: `as_directory`
aggiunge il separatore finale, `without_trailing_separator` lo toglie (una
radice lo tiene), `directory_part` e `last_component` dividono all'ultimo
separatore. `directory_entries` elenca una directory in ordine, con il
separatore dopo ogni sottodirectory, e `list-dir` può mettere prima le
directory. `file_completions` ne prende i nomi che continuano ciò che è stato
scritto, come percorsi interi, e i file nascosti solo dopo un punto: lo usa il
completamento del codice `f`, in `commands.rs`.

=== Il disco <disco>

Le primitive del disco espandono il percorso che ricevono. `directory-entry-count`,
`delete-file`, `rename-file` e `read-file-to-string`, quando non riescono,
rispondono nil e scrivono il motivo nel log. `delete-file` toglie un link
simbolico e mai ciò a cui punta, e una directory piena solo con RECURSIVE;
`rename-file` non sostituisce un file che c'è.

`walk_files` visita un albero in profondità, con una pila invece della
ricorsione, e restituisce i percorsi relativi alla radice. Non entra nelle
directory che PRUNE nomina, scarta i file che finiscono con uno dei suffissi,
si ferma al limite, e prima di leggere una directory chiede se continuare. La
usano `directory-files-recursive`, che paga un'unità di carburante per ogni
percorso che restituisce, e la ricerca nei file, sul suo thread.

=== L'ambiente <ambiente>

`getenv` e `setenv` leggono e cambiano le variabili d'ambiente del processo,
che i programmi avviati dopo ereditano. `setenv` rifiuta un nome vuoto, o con
`=` o un byte nullo, che farebbero andare in panic `set_var`. Un lock di `std`
mette in fila le letture di `std::env` e l'avvio dei processi con le scritture,
su qualunque thread; non copre il codice C che legge l'ambiente da sé
#rapporto("BKG-3"). `data-directory` cerca la directory dei dati accanto
all'eseguibile, e un livello sopra, dove la trovano i binari dei test.

=== Uscire <uscire>

`quit` offre di salvare, uno alla volta, i buffer per cui `has_unsaved_work`
risponde sì: modificati, scrivibili e con un file. Poi `quit--confirm` esce, o
chiede di scrivere `yes` se qualcosa è ancora da salvare. `save-some-buffers`
fa le stesse domande e alla fine chiama AFTER. Ogni domanda è una keymap
transitoria con quattro tasti, `y`, `n`, `!` e `q`, che chiamano
`save-some-buffers--answer` con il resto della lista; prima di ogni domanda il
buffer viene guardato di nuovo, perché una risposta esegue del Lisp che può
averlo salvato o chiuso. Salva `write_buffer`, in `files.rs`. `close-buffer`
chiede con la stessa funzione.

=== Le domande <domande>

Una domanda non restituisce la risposta: la primitiva mette la domanda sullo
schermo e torna, e la risposta arriva dopo, con un altro tasto, come chiamata
della callback che le è stata data. `y-or-n` installa una keymap transitoria in
cui rispondono solo `y` e `n` (`OnUnbound::Refuse`: un altro tasto non fa nulla
e non dice nulla); ogni tasto chiama `ask--answer`, che toglie la keymap prima
di chiamare la callback. `yes-or-no` apre il minibuffer e passa ciò che viene
scritto a `ask--typed`, che chiede di nuovo se non è né `yes` né `no`;
chiudere la domanda vale `no`. `ask_with_keys` è la stessa cosa con i tasti
scelti da chi chiama: la usa `save-some-buffers`.

=== I buffer <primitive-buffer>

`buffers.rs` crea, cambia, legge e chiude i buffer. `kill-buffer` è il comando,
e `close-buffer` la stessa funzione sotto un altro nome: un buffer con lavoro da
salvare chiede prima, con `yes-or-no`, e la risposta lo chiude con
`kill-buffer-without-saving`. `buffer-put` e `buffer-get` appendono a un buffer
un valore Lisp, nella tabella dei dati del buffer, sotto il nome `lisp:CHIAVE`.
`major-mode`, `buffer-modified-p` e `buffer-file-name` leggono il nome del
buffer allo stesso modo, quello corrente se manca.

=== Le finestre <primitive-finestre>

`windows.rs` divide e chiude le finestre (`split-window-below`,
`split-window-right`, `delete-window`, `delete-other-windows`,
`display-buffer-at-bottom` per una striscia in fondo alla cornice), sposta il
focus (`other-window`, `select-window`), scorre la vista (`scroll-up-command`,
`scroll-down-command`, `recenter`) e apre e chiude le flottanti
(`make-floating-window`, `close-floating-window`). Con una misura, una
divisione fissa le righe o le colonne della finestra divisa (`FirstFixed`), e
la nuova prende il resto. `close-floating-window` chiude sempre la flottante
più recente; senza flottanti mette il focus sulla finestra 0
#rapporto("CMD-8").

=== Gli argomenti <lettori-argomenti>

`args.rs` ha i lettori con cui le primitive leggono i loro argomenti: `text`,
una stringa; `name`, un simbolo o una stringa; `name_or_empty`, `name_or` e
`optional_name`, con il valore che danno quando l'argomento manca;
`movement_count` e `repeat_count`, i conteggi (@primitive-modifica); `quoted`,
che mette un valore in una forma da valutare dopo, come le callback delle
domande.

== Le decisioni <decisioni-primitive>

+ *Un file per concetto.* Le primitive di modifica stanno in tredici file, e
  ognuno ha il nome di ciò che contiene: `kill-word` si trova in `killing.rs`,
  accanto a `yank` e a `cut_out`, e `indent-line-to` in `indentation.rs`, il
  nome della pagina che lo descrive.
+ *Le porte prendono il `Buffer` intero, e il loro risultato non si ignora.*
  Registrare una modifica richiede la storia che sta accanto al testo, e
  prenderli insieme rende impossibile tenere l'uno senza l'altra. Il `bool` è
  `#[must_use]`: un chiamante non può supporre in silenzio che la modifica sia
  avvenuta e dire all'utente che è andata.
+ *Un messaggio solo per il rifiuto.* «Buffer is read-only», lo stesso per ogni
  primitiva: il motivo è sempre quello, e il nome del comando non direbbe
  nulla che l'utente non sappia, perché ha premuto il tasto. Il silenzio è ciò
  da evitare: un tasto che non fa nulla e non dice nulla sembra un editor
  rotto, non un buffer protetto #rapporto("BUF-4").
+ *Una modifica disattiva il mark, non lo cancella.* Ogni modifica passa dalle
  porte, quindi una modifica chiude la selezione, come in Emacs. La posizione
  resta per `exchange-point-and-mark`, e un mark attivo non ha mai avuto una
  modifica sotto: non va mai spostato.
+ *`changed` è una funzione sola.* Le porte, e i chiamanti di undo e redo, che
  scrivono sotto le porte, passano dalla stessa funzione per versione,
  colorazione, punti di ripresa ed evento `buffer-changed`. Una modifica che
  incrementasse la versione senza invalidare la colorazione, o il contrario, si
  vedrebbe solo sullo schermo, minuti dopo, come un colore che non si assesta.
+ *`insert` unisce i suoi argomenti.* `(insert a b c)` è una modifica sola: un
  passo di undo, e un cambiamento solo per l'evidenziatore.
+ *L'incolla normalizza i fine riga, `insert` no.* Un terminale in modalità
  bracketed paste consegna i byte della selezione, e un a capo arriva spesso
  come CR, ciò che manda Invio; l'indice delle righe conta solo `\n`, e dieci
  righe incollate sarebbero una riga lunga con nove caratteri invisibili. La
  conversione sta alla porta da cui passa ogni incolla, anche uno yank dagli
  appunti di sistema; un CR scritto da Lisp di proposito resta un CR. Su
  Windows la conversione non c'è #rapporto("CMD-7").
+ *Una kill restituisce il testo, e l'anello lo riceve dopo.* L'anello è un
  compartimento che non si prende mai con il lock di un buffer: prenderlo nella
  closure aggiungerebbe all'ordine dei lock un arco che nient'altro rispetta.
  Per questo `cut_out` restituisce il testo e ogni kill lo consegna fuori.
+ *Una kill rifiutata non mette nulla nell'anello.* Un testo nell'anello che è
  ancora nel buffer verrebbe duplicato dallo yank seguente. Lo stesso per
  `yank`: un'incollatura rifiutata non registra una posizione, che il
  `yank-pop` seguente userebbe per cancellare altro testo.
+ *I nomi e la distinzione sono quelli di Emacs.* `kill-*` mette nell'anello
  ciò che toglie, `delete-*` lo scarta: una configurazione scritta per Emacs
  trova i comandi che si aspetta, e si comportano come lì.
+ *Una cancellazione toglie ciò che il movimento attraversa.* Una kill non
  calcola da sé dove arriva: chiede alla funzione del movimento con lo stesso
  nome. Due calcoli separati finirebbero per divergere, e la differenza si
  vedrebbe solo come «a volte si mangia il trattino basso».
+ *La colonna obiettivo sopravvive a una serie di movimenti verticali.* Riletta
  dal punto a ogni movimento, una riga corta in mezzo la ridurrebbe per sempre:
  si scende oltre una riga corta, si risale, e non si torna dove si era. Si
  rilegge quando il comando precedente non era un movimento verticale
  (`last-command`), e si chiede prima del lock del buffer.
+ *Il punto come numero: funzioni, non comandi, in caratteri.* Nessuno arriva
  utilmente a `point` con `M-x`, mentre ogni movimento descritto è un comando.
  Una posizione fuori dal buffer va al bordo più vicino invece di essere
  rifiutata: è quasi sempre una posizione calcolata prima di una modifica, e
  mettere il punto in un posto vero serve più di un errore.
+ *Le espressioni si lessicano, le parole no.* Da fuori `forward-word` e
  `forward-sexp` si somigliano; dentro, una parola si trova guardando i
  caratteri, un'espressione lessicando il buffer con la tabella del modo, perché
  una parentesi è un delimitatore solo fuori da stringhe e commenti.
+ *L'indentazione: il meccanismo in Rust, la politica in Lisp.* La colonna di
  una riga è una domanda sulla lingua, e un modo nuovo deve poterla rispondere
  senza ricompilare; leggere e riscrivere l'indentazione attraverso le porte è
  meccanismo, e sta qui.
+ *Una riga vuota è indentata fino in fondo, e il punto tiene il suo
  carattere.* Così riporta Emacs, e così `indent-line-to` non conserva gli
  spazi in coda come testo. Il punto dentro l'indentazione va alla fine di
  quella nuova; oltre, tiene il carattere su cui stava, spostato della
  differenza.
+ *I verbi sono primitive.* Sono ciò che un editor #emph[è], come `kill-line`:
  un utente che toglie un file `.lisp` dalla sua configurazione non deve
  trovare `M-u` senza effetto.
+ *Ogni verbo dice dove lascia il punto.* È metà di ciò che rende un tasto
  ripetibile: `M-u M-u M-u` sale di tre parole perché ognuno lascia il punto
  dopo la parola che ha cambiato. I verbi degli spazi lo lasciano dov'erano gli
  spazi, così si continua a scrivere da lì.
+ *Una sostituzione che non cambia nulla non è una modifica.* Quando il testo è
  già quello che diventerebbe, `replace_span` sposta solo il punto. Non è
  un'ottimizzazione: una modifica che non cambia caratteri segnerebbe comunque
  il buffer come modificato, aggiungerebbe un passo di undo e sveglierebbe la
  colorazione, e mettere in maiuscolo ciò che lo è già lascerebbe un file da
  salvare.
+ *Senza regione, un errore.* Un comando che non fa nulla in silenzio quando
  l'utente credeva di avere una selezione è peggio di uno che lo dice; i
  rettangoli dicono lo stesso con le loro parole.
+ *`yank-pop` è un passo di undo.* Toglie e rimette attraverso le porte, nello
  stesso comando; il secondo passo non si tenta se il primo è stato rifiutato,
  che lascerebbe il testo nuovo in un buffer da cui quello vecchio non è uscito.
+ *Le modifiche si calcolano prima e si applicano dall'ultima.* Lavorare in
  avanti rende sbagliato ogni offset dopo la prima modifica, e lo sbaglio sembra
  plausibile: la prima riga viene giusta, la terza ha l'apertura in mezzo al
  testo. Ricalcolare a ogni passo funzionerebbe, con un lock e una lettura per
  riga. Il punto si rimette dallo stesso elenco.
+ *L'apertura va alla colonna della riga meno indentata.* Così un blocco
  commentato tiene la sua forma: aperture indentate con il codice fanno di un
  `if` annidato una scala di `//`. Le righe vuote dentro una regione restano
  vuote, perché un'apertura lascerebbe spazi in coda, e non contano per
  decidere se la regione è già commentata; una riga vuota da sola si commenta,
  perché è ciò che è stato chiesto.
+ *Una regione che finisce all'inizio di una riga non la prende.* Selezionare
  tre righe trascinando lungo il margine sinistro finisce sulla colonna 0 della
  quarta, che nessuno ha scelto. Vale per i commenti e per `Tab` su una
  regione.
+ *La grafia dei commenti la dice il modo.* Un commento è la sola sintassi che
  cambia con ogni lingua, e l'editor ne conosce già la grafia: niente qui decide
  come è fatto un commento, e un modo che non ha detto nulla lo sente dire,
  invece di ricevere un `#` o un `//` a caso.
+ *Il punto all'angolo in alto a sinistra.* È dove comincia il blocco appena
  toccato, da dove parte la prossima cosa da fargli, ed è la stessa risposta
  per tutti i comandi che cambiano il buffer. `copy-rectangle-as-kill` non lo
  sposta: il mark sopravvive a una copia, e il punto all'angolo andrebbe sopra
  il mark, lasciando una selezione senza larghezza; `M-w` fa lo stesso.
+ *Una riga corta si allunga per inserire, non per svuotare.* Una colonna che
  non esiste ancora è ciò che `open-rectangle`, `yank-rectangle` e
  `string-rectangle` devono riempire; `clear-rectangle` svuota ciò che c'è, e
  una riga corta non ha nulla da svuotare. Un blocco incollato arriva intero,
  con le righe che mancano in fondo al buffer, e senza spazi su una riga dove
  non porta nulla.
+ *`rectangle-mark-mode` spento torna una regione normale.* Premuto due volte,
  rende la selezione che c'era, non nessuna selezione; il flag sta sul mark, e
  non c'è una seconda cosa da ricordarsi di spegnere.
+ *I dati della corrispondenza hanno la forma di Emacs, in caratteri.* È la
  forma che ogni pezzo di Emacs Lisp si aspetta, e il codice portato funziona
  senza essere ristrutturato; il prezzo è `save-match-data`. Le posizioni sono
  caratteri, così una corrispondenza in una stringa con una `è` dà gli stessi
  numeri che in Emacs.
+ *I dati della corrispondenza sono per thread.* Un lavoro Lisp sul thread
  dell'esecutore e un comando sul thread principale non possono coordinarsi, e
  il primo non deve cambiare ciò che il secondo sta per leggere.
+ *Un'espressione sbagliata è un errore, non un nil.* Un'espressione che non
  compila è un difetto dell'espressione, e un nil la farebbe sembrare una
  stringa che non corrisponde. Una corrispondenza fallita invece lascia i dati
  com'erano.
+ *La forma di una voce è scritta una volta.* `scan-buffer` e le liste di
  risultati costruiscono le voci con le stesse funzioni, così una vista si
  scrive una volta e mostra risultati da ovunque. La lista è posizionale, come
  ogni risposta strutturata dell'editor; `KIND` distingue un buffer da un file,
  e `MATCH-START` e `MATCH-END` sono dentro `TEXT`, così una vista evidenzia
  senza cercare di nuovo.
+ *`scan-buffer` paga carburante per carattere percorso.* Un megabyte per
  un'unità permetterebbe a un ciclo un lavoro senza limite dentro un budget
  fatto per limitarlo; è la regola di `directory-files-recursive`, che paga
  un'unità per ogni percorso che restituisce.
+ *Un percorso è assoluto appena entra.* Un percorso relativo ha meno
  componenti di quanti sembri: l'elenco di `.` avrebbe l'intestazione `./`, e
  chiedere la directory sopra porterebbe alla radice, a un tasto da dove si era.
  Il rimedio non è insegnare a ogni chiamante a contare meglio: un percorso è
  ambiguo finché non è assoluto, e lo è solo nel momento in cui l'utente lo
  scrive.
+ *`..` si toglie per nome, senza chiedere al disco.* Attraverso un link
  simbolico le due risposte differiscono: per nome `/a/link/..` è `/a`, sul
  disco è il genitore della destinazione. Quella per nome è quella che vuole un
  elenco di file, dove `^` in `/a/link/` mostra `/a/`, da cui si veniva; ed è
  la sola possibile per un percorso che non esiste ancora, come la destinazione
  di una rinomina.
+ *I nomi dei file si compongono in Rust.* Che cosa sia un separatore e che
  cosa una radice lo dice la piattaforma: `/a/b` e `C:\a\b` hanno la stessa
  forma e nessun carattere in comune. Un modulo Lisp che spezza su `/` scrive la
  risposta di una piattaforma e la chiama regola.
+ *Un errore sul disco si riferisce, non si segnala.* Cancellare sei file non
  deve fermarsi al secondo perché uno è stato negato, lasciandone quattro e un
  elenco non aggiornato: chi chiama vuole sapere quali sono falliti, e
  continuare. Ogni primitiva risponde nil e scrive il motivo nel log, come
  `find-file` per un file che non legge.
+ *Le primitive del disco non chiedono, e non sono comandi.* Una conferma
  dipende dallo stato dell'editor, da una regione attiva, da un file aperto,
  dall'ultima risposta, e sta in Lisp con il resto dell'interfaccia; una
  primitiva che chiedesse non si potrebbe chiamare da uno script.
  `M-x delete-file` sarebbe un comando in cui solo il prompt separa un errore
  di battitura da un file cancellato: chi cancella usa il gestore di file
  (`dired.lisp`), che mostra che cosa c'è e chiede di conseguenza.
+ *La potatura sta in Rust, l'elenco arriva da Lisp.* Quali file interessino è
  politica, e sta in Lisp; non entrare in una directory è l'eccezione, perché
  non si può fare dopo: `.git` contiene migliaia di oggetti, e una visita che li
  raccogliesse spenderebbe il limite prima di arrivare a un file sorgente. Così
  anche i suffissi da scartare: filtrarli in Lisp costerebbe una valutazione per
  file e per suffisso.
+ *Una visita paga un'unità di carburante per percorso.* L'interprete conta
  un'unità per chiamata, giusto per un passo e sbagliato per una primitiva che
  restituisce una lista lunga quanto vuole: ventimila percorsi per tre unità. Il
  carburante deve limitare il tempo, e una visita che il contatore non vede fa
  un ciclo di visite che la guardia non ferma.
+ *Si chiede solo di un buffer modificato, scrivibile e con un file.* Un buffer
  senza un posto dove salvarlo non si salva rispondendo `no`: `*scratch*` e i
  buffer dell'aiuto e degli elenchi sono modificati di continuo, e chiedere di
  loro insegna a rispondere senza leggere. Uscire e chiudere un buffer usano la
  stessa funzione, così sono d'accordo su quali buffer contano.
+ *Due modi di chiedere.* Una risposta sbagliata non costa uguale ovunque: una
  domanda che sbagliando perde lavoro deve essere difficile da sbagliare
  (`yes-or-no`), e una fatta otto volte di fila non deve chiedere otto parole
  (`y-or-n`). Le domande di `save-some-buffers` sono tasti singoli; quella che
  perde il lavoro, alla fine di `quit`, si scrive per intero.
+ *Una domanda prende una callback.* L'editor non si blocca: un comando finisce
  e torna, e la risposta arriva dopo, in un altro comando, quando non c'è più
  nessuno a cui restituirla. In Lisp si legge come un `cond` diviso tra due
  lambda: è il prezzo di un editor che risponde mentre una domanda è sullo
  schermo.
+ *La callback sta nella forma, non in una variabile.* Ogni domanda costruisce
  la chiamata che le risponderà, `(ask--answer (quote CALLBACK))`, e la dà a una
  keymap o al minibuffer. Niente resta in un posto condiviso, e una domanda
  fatta dalla risposta a un'altra sono due forme, non due scritture nello stesso
  posto. Per lo stesso motivo `close-buffer` passa una lambda con dentro il
  nome del buffer.
+ *I dati appesi a un buffer stanno nel buffer.* Un modulo che deve ricordare
  qualcosa di un buffer avrebbe due alternative peggiori: una variabile globale
  con il nome del buffer per chiave, che sbaglia quando il buffer cambia nome e
  resta dopo che è chiuso, o il testo del buffer, che va bene solo per ciò che
  si mostrerebbe comunque. La chiave ha il prefisso `lisp:`, perché un valore di
  Lisp e uno di Rust con lo stesso nome non si scontrino.
+ *Una divisione con una misura fissa la finestra divisa.* Un elenco di file
  che vuole restare largo 40 colonne deve tenerle quando la cornice cambia
  misura, ed è il file accanto che deve crescere; ed è ciò che vuol dire
  `split-window-right SIZE` in Emacs.
+ *Un argomento che manca e uno sbagliato sono lo stesso errore.* Un lettore
  riceve un `Option`, ciò che dà `args.get(n)`, e riferisce un argomento
  assente come un nil del tipo sbagliato: chiamare senza niente dove va una
  stringa è sbagliato come chiamare con un numero, e «expected String, got nil»
  lo dice in una frase. Una primitiva per cui conta l'arità lo dice da sé, con
  `WrongNumberOfArguments`, perché è un'altra lamentela.
+ *I lettori leggono, non interpretano.* Nessun lettore sa che cosa significhi
  un argomento: non c'è un `pattern_arg` né un `prompt_arg`, perché un modello e
  un prompt sono tutti e due il primo argomento come stringa. Ciò che cambia tra
  i chiamanti è il messaggio d'errore, che è un parametro.

// ===========================================================================
= I moduli Lisp <moduli-lisp>
// ===========================================================================

Quasi tutto il comportamento visibile è in `core/lisp/`: le associazioni dei
tasti, i comandi di alto livello, i modi dei linguaggi, l'interfaccia dei
completamenti. `build.rs` copia i file in `target/<profilo>/data/lisp` a ogni
compilazione, da un #emph[elenco esplicito]: un modulo nuovo va aggiunto lì,
altrimenti non arriva accanto all'eseguibile e `eval-file` non lo trova.

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Modulo], [Che cosa aggiunge]),
  [`commands`], [`defcommand`, `save-match-data` e altre macro di base.],
  [`debug`], [`message`, `report-error`, il buffer `*Backtrace*`, `debug-on-error`.],
  [`common-keymaps`], [Le associazioni di Emacs: movimento, kill, file, buffer, finestre, `C-g`.],
  [`indent`], [Tab, `indent-region`, `newline-and-indent`, `electric-indent-mode`.],
  [`minibuffer`], [Tasti e comportamenti del prompt.],
  [`rust-mode`, `risp-mode`, `cc-mode`], [Modi per Rust, per il Lisp dell'editor e per C/C++.],
  [`dired`], [Una directory in un buffer (`C-x d`).],
  [`completion`], [La striscia dei completamenti (`*completion-read-function*`).],
  [`preview`], [Anteprima di `string-rectangle`.],
  [`clipboard`], [Copia le uccisioni negli appunti di sistema.],
  [`electric-pair`], [Chiusura automatica di parentesi e virgolette.],
  [`buffer-list`], [`C-x b` e `C-x C-b`.],
  [`shell`], [`M-!`.],
  [`manpage`], [Pagine di manuale.],
  [`help`], [`C-h f`, `C-h k`, `C-h b`, `describe-prefix-keys`.],
  [`occur`], [`M-s o`, `M-s g`.],
  [`compile`], [`C-c c` e `M-g n`.],
  [`theme`], [`C-c t`: scelta del tema.],
  [`completion-at-point`], [Le cinque fonti di `C-M-i`.],
)

Le variabili che l'editor legge da Lisp:

#ref-table(
  columns: (4.6cm, 2.2cm, 1fr),
  header: ([Variabile], [Default], [Effetto]),
  [`echo-message-timeout`], [5], [Secondi di permanenza di un messaggio; `nil` per sempre.],
  [`minibuffer-width`, `minibuffer-height`], [60, 3], [Dimensioni del prompt.],
  [`mode-line-format`], [vedi sopra], [Riga di stato.],
  [`window-separator`], [`"│"`], [Carattere tra finestre; `nil` per uno spazio.],
  [`display-line-numbers`], [`nil`], [`t` assoluti, `'relative` relativi.],
  [`display-line-numbers-width`], [3], [Cifre minime del margine.],
  [`mouse-mode`], [`nil` (`t` in `init.lisp`)], [Il mouse all'editor invece che al terminale.],
  [`after-resize-hook`], [`nil`], [Funzioni chiamate con larghezza e altezza.],
  [`frame-width`, `frame-height`], [---], [Dimensioni correnti, scritte dal front-end.],
  [`lisp-path`], [`data/lisp`], [Dove `eval-file` cerca i moduli.],
  [`help-key`], [`"C-h"`], [Il tasto che, dopo un prefisso, chiede che cosa lo segue.],
  [`worker-fuel`], [200.000], [Passi per turno di un worker.],
  [`watch-files`, `watch-file-interval`], [`t`, 3], [Il watcher dei file.],
  [`auto-save`, `auto-save-interval`, `auto-save-directory`], [`t`, 30, ---], [L'autosalvataggio.],
  [`history-length`], [100], [Voci per storia di prompt.],
  [`*minibuffer-read-function*`], [`default-minibuffer-prompt`], [Chi apre i prompt.],
  [`*key-capture-function*`], [`nil`], [Chi riceve la prossima sequenza invece di eseguirla.],
)

// ===========================================================================
= I test <test>
// ===========================================================================

I test dell'editor stanno in `core/src/tests/`, uno o più file per
funzionalità (circa ottanta), più `tests/perf/` per le misure e i test del
front-end in `src/tests/`. Costruiscono un editor con `create_global_env`,
caricano i moduli Lisp che servono con `include_str!` e valutano Lisp o
simulano tasti con `handle_key_event`. `isolate_config_for_tests` punta
`RSEDIT_CONFIG_DIR` a una directory temporanea del processo, una volta sola,
così i test non scrivono nella configurazione di chi li esegue.

Alcuni file meritano di essere conosciuti prima di cambiare il codice che
proteggono: `layering_tests` (i due confini), `deadlock_tests` (i lock
attraverso Lisp), `doc_faithfulness_tests` e `primitive_shape_tests` (le
docstring), `edit_protection_tests` (sola lettura), `undo_tests`,
`frame_snapshot_tests`, `shipped_lisp_tests` (ogni modulo distribuito, caricato
nell'ordine di `init.lisp`: nell'editor reale un errore di caricamento finisce
solo nel log). `perf::suite` misura tempi reali con più thread e può fallire su
macchine lente o condivise #rapporto("DOC-4").

// ===========================================================================
= Ricette <ricette>
// ===========================================================================

== Una primitiva o un comando in Rust

+ Scegli il modulo di `primitives/`; scrivi la docstring come `const`, con la
  firma nella prima riga.
+ Scrivi la funzione con `primitive!`. Leggi lo stato con le closure della
  facciata (`with_current_buffer_mut`, `windows`, …) e #emph[non] chiamare Lisp
  con un lock preso: copia, rilascia, poi chiama.
+ Modifica il testo solo con `insert_text` e `delete_range`, e controlla il
  loro risultato.
+ Se scorri qualcosa di grande, addebita il carburante (`ctx.consume_fuel`).
+ Registra con `into.function` o, per un comando, con `into.command` e le
  specifiche degli argomenti.
+ Aggiungi i test in `core/src/tests/` e aggiorna la tabella di @primitive.

== Un comando in Lisp

```lisp
(defcommand count-words-region (start end) ("r")
  "Report how many words the region holds."
  (message "%d words" (length (split-string (buffer-substring start end)))))
(define-key nil "M-=" 'count-words-region)
```

`define-key` con `nil` lega nella keymap globale, con un simbolo di modo nella
keymap di quel modo. Un comando legato senza argomenti passa da
`call-interactively`, che gli chiede la regione.

== Un modo maggiore

Il modello è `rust-mode.lisp`:

```lisp
(make-mode 'toy-mode)
(add-syntax-region 'toy-mode "\"" "\"" 'string "\\\\.")
(add-syntax-rule 'toy-mode "#.*" 'comment)
(add-syntax-rule 'toy-mode "\\b(let|if|else)\\b" 'keyword)
(set-comment-syntax 'toy-mode '(("#")))
(put 'toy-mode 'tab-width 2)
(add-auto-mode "\\.toy$" 'toy-mode)
```

Le regioni prima delle regole; ciò che può attraversare una riga è una regione,
il resto una regola. Ricorda che `make-mode` sostituisce un modo esistente.

== Un lavoro di sfondo

In Lisp: `background-call` con una fiber che fa `(yield)` tra un pezzo e
l'altro e una callback `ON-DONE` per il risultato, oppure `define-worker` con
una fiber che aspetta il suo argomento, `(yield '(wait TOPIC))`. La pagina
`background` ha gli esempi, e le regole che un lavoro segue sono in
@lavori-lisp.

In Rust:

+ Scrivi un tipo che implementa `Job<EditorState<B>>`. Ogni `step` fa una
  quantità limitata di lavoro: copia sotto lock, lavora senza, ricontrolla la
  versione prima di salvare. Dichiara le unità con `cx.worked`, e risponde
  `Continue` se resta altro, `Wait` o `Sleep` se no. Il modello è
  `Highlighter` (@utenti-sfondo).
+ Dagli una `Spec`: un nome unico, la classe (`High` solo se l'utente aspetta
  di vederne il risultato), l'unità, gli argomenti che lo svegliano, e
  `Start::After` se il primo passo non serve subito.
+ Sottomettilo con `work().submit(spec, Box::new(lavoro))` dove lo stato
  esiste: in `EditorState::new`, o in `create_global_env` se legge
  impostazioni del Lisp, che allora rilegge a ogni passo.
+ Ciò che può bloccarsi non è un lavoro: va su un thread dedicato, con
  `spawn_work`, o con `spawn_writer` se scrive in un buffer, e con
  `Stop::on_cancel` per l'azione che lo sblocca.
+ Aggiungi la riga alla tabella di @utenti-sfondo, e i test in modalità
  manuale (@prove-sfondo).

== Un modulo Lisp nuovo

Scrivi `core/lisp/nome.lisp`, aggiungilo all'elenco di `core/build.rs`, aggiungi
`(eval-file "nome")` a `DEFAULT_INIT_LISP` in `editor/boot.rs` (e, a mano, al
tuo `init.lisp`), e un test che lo carichi.

== Un nuovo front-end

Un front-end deve: tradurre gli eventi nei tipi di `rsedit_core`; chiamare
`handle_key_event`, `handle_paste`, `handle_mouse_event` e `resize`; comunicare
`frame-width` e `frame-height`; in un ciclo, chiedere `snapshot` e disegnarlo
senza tenere nulla dell'editor; aspettare al massimo `next_redraw_in` (e
`drag_scroll_in`) e chiamare `tick` quando l'attesa scade; mettere
`frame.clipboard` negli appunti. Tutto ciò che c'è da sapere su colori, bordi e
cursore è nella fotografia; `src/tui.rs` è l'esempio completo.

// ===========================================================================
= Appendice
// ===========================================================================

== Glossario

/ Buffer: testo con punto, mark, storia e modo; può visitare un file.
/ Punto: la posizione del cursore in un buffer, in caratteri.
/ Mark, regione: una seconda posizione; la regione è tra mark attivo e punto.
/ Comando: una funzione registrata con le sue specifiche di argomenti.
/ Keymap transitoria: una keymap consultata prima di tutte, finché non viene tolta.
/ Compartimento: un gruppo di campi dello stato sotto un solo lock, in `managers/`.
/ Facciata: `EditorState`, l'unico che tiene più lock di compartimento insieme.
/ Fotografia: il `FrameSnapshot`, ciò che il front-end disegna.
/ Lavoro: ciò che il WorkManager esegue a passi; implementa `Job`. Nel codice è un #emph[job]\; #emph[work] è un lavoro o un thread dedicato, e #emph[worker] resta solo nei nomi del Lisp (`define-worker`, `stop-worker`, `running-workers`, `worker-fuel`).
/ Passo: una chiamata a `Job::step`, cioè una quantità limitata di lavoro.
/ Argomento: un evento con un nome e senza dati, cioè un contatore che cresce a ogni emissione.
/ Esecutore: il thread del WorkManager che esegue i passi, uno alla volta.
/ Thread dedicato: un thread del WorkManager per un lavoro che può bloccarsi, fermato con il suo `Stop`.
/ Watchdog: il thread del WorkManager che osserva il passo in corso e scrive il file di log.
/ Callback dovuta: una chiamata chiesta da un lavoro di sfondo ed eseguita dal thread dei comandi.
/ Epoca: il timbro del comando corrente, con cui l'undo raggruppa le modifiche.

== Mantenere aggiornato questo manuale

- Nuovo campo di `EditorState` o nuovo compartimento → @stato (tabelle e ordine
  dei lock).
- Cambiamento del percorso di un tasto → @tasto; di `call-interactively` →
  @comandi.
- Nuova primitiva o nuovo modulo di `primitives/` → @primitive e l'elenco in
  appendice (lo script che lo genera cerca `into.function(` e `into.command(`).
- Nuovo lavoro dell'editor → la tabella di @utenti-sfondo; nuova opzione o
  nuova segnalazione → @misure.
- Nuovo campo della fotografia o cambiamento del renderer → @frame.
- Nuovo modulo Lisp o nuova variabile letta da Rust → @moduli-lisp.
- Difetto corretto → aggiorna la descrizione, togli il rimando al rapporto e la
  scheda da `rapporto-problemi.typ`.

Gli estratti di codice vanno riletti quando cambia la funzione da cui vengono.
I diagrammi usano gli aiuti di `style.typ` descritti nel manuale
dell'interprete. Dopo ogni modifica conviene compilare e guardare le pagine.

== Elenco delle primitive dell'editor <elenco-primitive>

Generato da `into.function(` e `into.command(` nei file di `primitives/`. Un asterisco indica un comando, raggiungibile con `M-x`.

#ref-table(
  columns: (3.2cm, 1fr),
  header: ([Modulo], [Primitive]),
  [`ask` (4)], [`y-or-n`, `yes-or-no`, `ask--answer`, `ask--typed`],
  [`buffers` (18)], [`buffer-put`, `buffer-get`, `buffer-substring`, `switch-to-buffer`, `current-buffer`, `buffer-create`, `close-buffer`, `kill-buffer`\*, `kill-buffer-without-saving`\*, `buffer-string`, `clear-buffer`, `with-current-buffer`, `set-buffer-read-only`, `buffer-read-only-p`, `buffer-modified-p`, `buffer-file-name`, `major-mode`, `set-major-mode`],
  [`commands` (10)], [`register-command`, `commandp`, `command-args`, `all-commands`, `execute-extended-command`, `command-completions`, `command-execute-prompt`\*, `all-buffer-names`, `call-interactively`, `pending-command`],
  [`comments` (6)], [`comment-dwim`\*, `comment-line`\*, `comment-indent`\*, `comment-region`\*, `uncomment-region`\*, `comment-syntax`],
  [`completion` (8)], [`completion-at-point`\*, `fuzzy-filter`, `completion-at-point-choose`, `add-completion-function`, `set-completion-functions`, `completion-functions`, `buffer-words`, `bounds-of-thing-at-point`],
  [`edits` (7)], [`delete-region`, `self-insert`, `insert`, `insert-pasted-text`\*, `insert-newline`\*, `delete-backward-char`\*, `delete-char`\*],
  [`environment` (4)], [`data-directory`, `getenv`, `setenv`, `path-separator`],
  [`file_names` (7)], [`list-dir`, `match-list`, `expand-file-name`, `file-name-as-directory`, `directory-file-name`, `file-name-directory`, `file-name-nondirectory`],
  [`files` (8)], [`find-file`\*, `save-buffer`\*, `save-buffer--overwrite`, `recover-file`\*, `recover-file--adopt`, `revert-buffer`\*, `revert-buffer--reread`, `write-file`\*],
  [`filesystem` (7)], [`file-exists-p`, `file-directory-p`, `directory-entry-count`, `delete-file`, `rename-file`, `directory-files-recursive`, `read-file-to-string`],
  [`general` (13)], [`eval-file`, `eval-string-top-level`, `define-key`, `set-local-keymap`, `define-repeat-key`, `repeat`\*, `log`, `regexp-opt`, `all-logs`, `backtrace`, `set-echo-message`, `set-transient-keymap`, `clear-transient-keymap`],
  [`help` (7)], [`key-binding`, `keys-with-prefix`, `keymap-bindings`, `where-is`, `command-specs`, `help--macro-doc`, `read-key-sequence`],
  [`indentation` (3)], [`current-indentation`, `previous-indentation`, `indent-line-to`],
  [`isearch` (10)], [`isearch-forward`\*, `isearch-backward`\*, `isearch-forward-regexp`\*, `isearch-backward-regexp`\*, `isearch-update`, `isearch-exit`, `isearch-abort`, `isearch-repeat-forward`, `isearch-repeat-backward`, `isearch-active-p`],
  [`killing` (14)], [`kill-line`\*, `kill-whole-line`\*, `kill-word`\*, `backward-kill-word`\*, `kill-paragraph`\*, `backward-kill-paragraph`\*, `kill-region`\*, `kill-ring-save`\*, `yank`\*, `yank-pop`\*, `set-kill-ring-max`\*, `kill-new`, `current-kill`, `kill-ring-length`],
  [`macros` (12)], [`kmacro-start-macro`\*, `kmacro-end-macro`\*, `kmacro-cancel-macro`\*, `kmacro-call-macro`\*, `kmacro-end-and-call-macro`\*, `kmacro-insert-counter`\*, `kmacro-set-counter`\*, `kmacro-counter`, `kmacro-recording-p`, `kbd-macro-keys`, `define-kbd-macro`, `kmacro-call-keys`],
  [`match_data` (7)], [`string-match`, `string-match-p`, `match-beginning`, `match-end`, `match-string`, `match-data`, `set-match-data`],
  [`minibuffer` (12)], [`minibuffer-cleanup`, `minibuffer-confirm`, `minibuffer-cancel`, `minibuffer-complete`, `minibuffer-choose-completion`, `default-minibuffer-prompt`, `minibuffer-read`, `completion-invalidate`, `minibuffer-history`, `clear-minibuffer-history`, `history-previous`\*, `history-next`\*],
  [`modes` (16)], [`make-mode`, `add-hook`, `remove-hook`, `add-syntax-rule`, `add-syntax-region`, `add-auto-mode`, `syntax-ppss`, `balance-point`, `bounds-of-enclosing-list`, `set-syntax-pairs`, `set-string-syntax`, `set-char-quote`, `set-syntax-entry`, `set-comment-syntax`, `syntax-class`, `matching-delimiter`],
  [`mouse` (6)], [`mouse-set-point`\*, `mouse-start-selection`\*, `mouse-drag-to`\*, `mouse-resize`\*, `mouse-mode-toggle`\*, `mouse-scroll`\*],
  [`movement` (20)], [`backward-char`\*, `forward-char`\*, `previous-line`\*, `next-line`\*, `beginning-of-line`\*, `end-of-line`\*, `forward-word`\*, `backward-word`\*, `forward-paragraph`\*, `backward-paragraph`\*, `beginning-of-buffer`\*, `end-of-buffer`\*, `goto-line`\*, `point`, `point-min`, `point-max`, `goto-char`, `line-number-at-point`, `current-line`, `current-column`],
  [`overlays` (5)], [`make-overlay`, `delete-overlay`, `remove-overlays`, `overlays-at`, `overlay-face`],
  [`quit` (5)], [`quit`\*, `quit-without-saving`\*, `save-some-buffers`\*, `save-some-buffers--answer`, `quit--confirm`],
  [`rectangle` (10)], [`rectangle-mark-mode`\*, `kill-rectangle`\*, `delete-rectangle`\*, `copy-rectangle-as-kill`\*, `yank-rectangle`\*, `open-rectangle`\*, `clear-rectangle`\*, `string-rectangle`\*, `rectangle-bounds`, `killed-rectangle`],
  [`region` (9)], [`set-mark`\*, `keyboard-quit`\*, `deactivate-mark`\*, `exchange-point-and-mark`\*, `mark-whole-buffer`\*, `mark`, `use-region-p`, `region-beginning`, `region-end`],
  [`replace` (12)], [`query-replace`\*, `query-replace-regexp`\*, `replace-string`\*, `replace-regexp`\*, `replace-this`, `replace-skip`, `replace-rest`, `replace-back`, `replace-done`, `replace-abandon`, `replace-match`, `replace-count`],
  [`results` (13)], [`occur--scan`, `grep--scan`, `next-error`\*, `previous-error`\*, `results-count`, `results-entry`, `results-state`, `results-visit`, `results-buffer`, `results-select`\*, `results-put`, `results-display`, `results-quit`\*],
  [`scan` (1)], [`scan-buffer`],
  [`sexp` (7)], [`forward-sexp`\*, `backward-sexp`\*, `kill-sexp`\*, `backward-kill-sexp`\*, `up-list`\*, `backward-up-list`\*, `down-list`\*],
  [`shell` (4)], [`shell-command-start`, `shell-command-to-string`, `parse-overstrike`, `shell-command-running-p`],
  [`theme` (6)], [`set-face`\*, `define-face`, `face-style`, `reset-faces`, `list-faces`, `list-colors`],
  [`undo` (5)], [`undo-join`, `undo`\*, `redo`\*, `undo-boundary`\*, `set-undo-limit`\*],
  [`verbs` (14)], [`upcase-word`\*, `downcase-word`\*, `capitalize-word`\*, `delete-horizontal-space`\*, `just-one-space`\*, `delete-blank-lines`\*, `back-to-indentation`\*, `join-line`\*, `duplicate-line`\*, `transpose-lines`\*, `transpose-chars`\*, `transpose-words`\*, `zap-to-char`\*, `zap-to-char--do`],
  [`virtual_text` (4)], [`make-virtual-text`, `clear-virtual-text`, `delete-virtual-text`, `virtual-text-at`],
  [`windows` (15)], [`split-window-below`\*, `split-window-right`\*, `delete-window`\*, `delete-other-windows`\*, `other-window`\*, `selected-window`, `select-window`, `count-windows`, `scroll-up-command`\*, `scroll-down-command`\*, `recenter`\*, `display-buffer-at-bottom`, `window-buffer`, `make-floating-window`, `close-floating-window`],
  [`workers` (9)], [`define-worker`, `stop-worker`, `running-workers`, `background-call`, `emit-event`, `work-option`, `set-work-option`, `work-options`, `work-list`],
)
