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

Questo manuale descrive la libreria `rsedit_core` (`core/src/`, esclusa la
cartella `lisp/`) e il front-end a terminale che la usa (`src/`). È il
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
  Due confini reggono l'architettura, e sono verificati dai test
  (`tests::layering_tests`): `lisp/` non nomina alcun tipo dell'editor, e `ui/`
  non nomina nulla di `editor/`. Il primo permette di provare e riusare
  l'interprete da solo; il secondo permette di scrivere un renderer contro una
  fotografia invece che contro gli interni della libreria.
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
e mai quelli sotto. La figura li mostra dal basso verso l'alto, cioè nella
direzione in cui si costruisce.

#canvas(height: 8.6cm)[
  #node(8.1cm, 0.4cm, 15.8cm, 0.7cm, [`src/` --- il front-end TUI (crossterm): traduce gli eventi, disegna la fotografia], fill: accent.lighten(85%))
  #node(8.1cm, 1.4cm, 15.8cm, 0.7cm, [`ui/` --- la descrizione dello schermo: finestre, layout, facce, `FrameSnapshot`. Non disegna nulla])
  #node(8.1cm, 2.4cm, 15.8cm, 0.7cm, [`background/` --- su quale thread gira il lavoro, e la regola che lo decide])
  #node(4.0cm, 3.4cm, 7.6cm, 0.7cm, [`feature/` --- minibuffer e isearch])
  #node(12.15cm, 3.4cm, 7.7cm, 0.7cm, [`modes/` --- modi e lavori che li servono])
  #node(8.1cm, 4.4cm, 15.8cm, 0.7cm, [`primitives/` --- tutto ciò che Lisp può chiamare (304 primitive)])
  #node(8.1cm, 5.4cm, 15.8cm, 0.7cm, [`editor/` --- `EditorState`, la facciata: l'unico che tiene due lock di compartimento insieme], fill: paper)
  #node(8.1cm, 6.4cm, 15.8cm, 0.7cm, [`managers/` --- lo stato, in compartimenti, ognuno sotto un lock], fill: paper)
  #node(4.0cm, 7.4cm, 7.6cm, 0.7cm, [`input.rs`, `commands.rs` --- tasti, keymap, comandi])
  #node(12.15cm, 7.4cm, 7.7cm, 0.7cm, [`text/` --- ricerca, rettangoli, kill ring])
  #node(4.0cm, 8.3cm, 7.6cm, 0.6cm, [`lisp/` --- l'interprete], fill: paper)
  #node(12.15cm, 8.3cm, 7.7cm, 0.6cm, [`buffer/` --- testo, undo, cache], fill: paper)
]

Le frecce sono omesse perché la regola è una sola: ogni livello può usare tutti
quelli disegnati sotto di lui. `lisp/` e `buffer/` non si conoscono; si
incontrano solo in `editor/`.

== Dove sta cosa

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Percorso], [Contenuto]),
  [`lib.rs`], [Ordine dei moduli e superficie pubblica (`pub use`). Tutto il resto è `pub(crate)`.],
  [`buffer/`], [`Buffer`, `BufferTrait`, `GapBuffer`, undo, mark, overlay, testo virtuale, cache di colorazione (`syntax.rs`) e di scansione (`scan.rs`), timbro del file su disco (`disk.rs`).],
  [`text/`], [Ciò che si può calcolare sul testo conoscendo solo `BufferTrait`: ricerca e isearch, rettangoli, kill ring, liste di posizioni (`results.rs`).],
  [`input.rs`], [`KeyEvent`, `KeyCode`, `MouseEvent`, `Keymap`, `TransientKeymap`, il parser dei nomi dei tasti, le keymap predefinite.],
  [`commands.rs`], [`ArgSpec`, `PrefixArg`, `Invocation`, `PendingCommand`: il vocabolario dei comandi.],
  [`managers/`], [I compartimenti dello stato: `Buffers`, `Windows`, `Modes`, `Commands`, `KillYank`, `Macros`, `History`, `Log`, `Runtime`.],
  [`editor/`], [`EditorState` e i suoi metodi, divisi per compartimento (`buffers.rs`, `windows.rs`...) o per preoccupazione (`keys.rs`, `mouse.rs`, `frame.rs`, `boot.rs`, `settings.rs`, `background.rs`, `macros.rs`).],
  [`primitives/`], [Le 304 primitive dell'editor, in 28 moduli, e `install_primitives`.],
  [`modes/`], [`MajorMode`, grammatica di colorazione (`syntax.rs`), tabella sintattica e scanner di espressioni (`sexp.rs`), i lavori di sfondo `Highlighter`, `Prescanner`, `FileWatcher`, `AutoSaver`.],
  [`feature/`], [Le due funzionalità che non possono dipendere da un file Lisp: il minibuffer e la ricerca incrementale.],
  [`background/`], [Lo scheduler, `ScheduledTask`, `ImmediateTask`, i worker Lisp e `background-call`.],
  [`ui/`], [Albero delle finestre, finestre flottanti, layout delle righe, facce e temi, `FrameSnapshot`.],
  [`tests/`], [Circa ottanta file di test dell'editor, più `perf/`.],
  [`../lisp/`], [I moduli Lisp distribuiti con l'editor (venti file).],
  [`../build.rs`], [Copia i moduli Lisp in `target/<profilo>/data/lisp`, accanto all'eseguibile.],
  [`../../src/`], [Il front-end: `main.rs` (avvio) e `tui.rs` (ciclo degli eventi e disegno).],
)

== La superficie pubblica

Un front-end vede soltanto ciò che `lib.rs` riesporta: `EditorState`,
`create_global_env`, `BufferTrait`, `GapBuffer`, i tipi di input (`KeyEvent`,
`KeyCode`, `KeyModifiers`, `MouseEvent`, `MouseKind`, `MouseButton`), i tipi
della fotografia (`FrameSnapshot`, `RenderableWindowView`, `Highlight`,
`GutterCell`, `Separator`, `Rect`, `Face`, `Style`, `Color`, `Theme`,
`NAMED_COLORS`), dall'interprete `Env`, `LispContext`, `Parser` ed `eval`, più
`mouse_mode`, `MOUSE_MODE`, `CONFIG_DIR`, `XDG_CONFIG_HOME` e l'alias
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
  [`is_running()`], [Falso dopo `quit`.],
  [`enable_log_file`, `log_diagnostic`, `set_echo_message`], [Diagnostica.],
)

== I thread

L'editor usa più thread, ma uno solo esegue i comandi. La figura mostra chi
esiste e che cosa condividono.

#canvas(height: 6.2cm)[
  #group(0.1cm, 0.3cm, 7.6cm, 5.6cm, [thread principale (comandi)])
  #node(3.9cm, 1.2cm, 6.8cm, 0.8cm, [ciclo degli eventi della TUI], fill: accent.lighten(85%))
  #node(3.9cm, 2.4cm, 6.8cm, 0.8cm, [`handle_key_event` → `run_command_form`])
  #node(3.9cm, 3.6cm, 6.8cm, 0.8cm, [hook, callback dovute (`tick`), `snapshot`])
  #node(3.9cm, 4.9cm, 6.8cm, 1.0cm, [l'unico thread che apre prompt\ e su cui girano i comandi], fill: paper)

  #group(8.5cm, 0.3cm, 7.6cm, 2.7cm, [thread dello scheduler (condiviso)])
  #node(12.3cm, 1.2cm, 6.8cm, 0.8cm, [`Highlighter`, `Prescanner` (ogni 40 ms)])
  #node(12.3cm, 2.3cm, 6.8cm, 0.8cm, [`FileWatcher`, `AutoSaver`, worker Lisp, job a turni])

  #group(8.5cm, 3.4cm, 7.6cm, 2.5cm, [un thread per lavoro (`RunNow`)])
  #node(12.3cm, 4.2cm, 6.8cm, 0.8cm, [comandi di shell (`ShellTask`), ricerca nell'albero (`TreeSearch`)])
  #node(12.3cm, 5.25cm, 6.8cm, 0.7cm, [e ogni `(spawn ...)` Lisp], fill: paper)
  #carrow((7.3cm, 2.4cm), (8.9cm, 1.6cm), colour: accent)
]

La freccia è la mailbox (`worker_mailbox`) con cui il thread dei comandi
consegna lavoro allo scheduler. Tutti condividono lo stesso `EditorState` (clonato: ogni campo è un `Arc`) e lo
stesso ambiente Lisp. Il carburante, il permesso di `yield`, il contrassegno
del comando per l'undo e i dati di `string-match` sono invece per thread
(variabili `thread_local!`). I lavori di sfondo non chiamano mai Lisp sul
proprio thread quando l'effetto deve toccare l'interfaccia: lasciano una
#emph[callback dovuta], che il thread principale esegue tra un comando e
l'altro (@sfondo).

// ===========================================================================
= Il ciclo di vita del programma <ciclo-vita>
// ===========================================================================

== Dal `main` al ciclo degli eventi

`src/main.rs` fa quattro cose: chiama `create_global_env::<GapBuffer>()`,
attiva il file di log `rsedit.log` #emph[nella directory corrente], apre il
file passato come primo argomento valutando `(find-file PERCORSO)` e, se lo
standard input è un terminale, entra in `tui_main`. Un errore di avvio
dell'interprete viene stampato e il programma esce; un errore di `find-file`
finisce nell'area dei messaggi.

#canvas(height: 8.4cm)[
  #lane(1.5cm, 0cm, 8.3cm, [`main.rs`], fill: accent.lighten(85%))
  #lane(5.2cm, 0cm, 8.3cm, [`create_global_env`], w: 3.2cm)
  #lane(8.9cm, 0cm, 8.3cm, [`EditorState`])
  #lane(11.7cm, 0cm, 8.3cm, [scheduler], fill: accent.lighten(85%))
  #lane(14.6cm, 0cm, 8.3cm, [Lisp])

  #msg(1.5cm, 5.2cm, 1.2cm, [chiamata])
  #msg(5.2cm, 8.9cm, 1.65cm, [`EditorState::new()`])
  #msg(8.9cm, 11.7cm, 2.1cm, [`spawn` + `Highlighter`, `Prescanner`], colour: accent)
  #msg(5.2cm, 14.6cm, 2.6cm, [`bootstrap_vm`: radice + 86 primitive])
  #msg(5.2cm, 14.6cm, 3.1cm, [`lisp-path`, variabili di impostazione])
  #msg(5.2cm, 14.6cm, 3.6cm, [`install_primitives`, minibuffer, isearch])
  #seq-note(5.4cm, 3.85cm)[`init.lisp` scritto se manca]
  #msg(5.2cm, 11.7cm, 4.75cm, [`FileWatcher`, `AutoSaver`], colour: accent)
  #msg(5.2cm, 14.6cm, 5.25cm, [`eval_file(init.lisp)` → i moduli])
  #msg(5.2cm, 1.5cm, 5.75cm, [`(state, env)`], dash: "dashed")
  #msg(1.5cm, 14.6cm, 6.3cm, [`(find-file ARGV[1])`])
  #self-msg(1.5cm, 6.85cm, [`tui_main`: raw mode, incolla, colori])
]

=== I passi di `create_global_env`

+ `EditorState::new()` riempie la keymap globale con le poche associazioni
  scritte in Rust (`fill_default_keymaps`), crea ogni compartimento con il suo
  `Arc<RwLock<…>>`, avvia lo scheduler di sfondo e gli consegna i due lavori
  che girano sempre: `Highlighter` e `Prescanner`, ogni 40 ms.
+ `bootstrap_vm` crea l'ambiente radice con le 86 primitive di base e
  verifica l'interprete.
+ `lisp-path` diventa `("<directory dell'eseguibile>/data/lisp")`.
+ Le impostazioni ricevono il valore iniziale, così che `(setq ...)` in una
  configurazione modifichi un valore che esiste già: `mouse-mode` (`nil`),
  `after-resize-hook` (`nil`), `echo-message-timeout` (5),
  `minibuffer-width` (60), `minibuffer-height` (3), `mode-line-format`,
  `window-separator` (`│`). Viene creato il modo `fundamental-mode`.
+ `install_primitives`, `install_minibuffer` e `install_isearch` registrano le
  primitive, i comandi, il modo `minibuffer-mode` e il modo `isearch-mode`.
+ `rsedit-path` riceve il percorso dell'eseguibile.
+ Se manca, `init.lisp` viene creato con `DEFAULT_INIT_LISP` (vedi sotto).
+ Lo scheduler riceve `FileWatcher` e `AutoSaver`, con gli intervalli letti
  dalle variabili (`watch-file-interval`, `auto-save-interval`).
+ `eval_file` valuta `init.lisp`, che carica i moduli Lisp.

=== Dove sta la configurazione

#ref-table(
  columns: (1fr, 1.4fr),
  header: ([Fonte, in ordine di precedenza], [Directory]),
  [`RSEDIT_CONFIG_DIR`], [usata così com'è, su ogni piattaforma],
  [`XDG_CONFIG_HOME` (solo Unix)], [`$XDG_CONFIG_HOME/rsedit`],
  [la piattaforma], [`%APPDATA%\rsedit` su Windows, `$HOME/.config/rsedit` altrove (anche su macOS)],
)

Il file `init.lisp` predefinito carica, in quest'ordine: `commands`, `debug`,
`common-keymaps`, `indent`, `minibuffer`, poi i moduli facoltativi
`rust-mode`, `risp-mode`, `cc-mode`, `dired`, `completion`, `preview`,
`clipboard`, `electric-pair`, `buffer-list`, `shell`, `manpage`, `help`,
`occur`, `compile`, `theme`, `completion-at-point`, e infine imposta
`dired-sort` a `'type` e `mouse-mode` a `t`. L'ordine conta: `commands`
definisce `defcommand`, con cui sono scritti gli altri, e `debug` definisce
`message` e `report-error`.

Il file viene scritto solo se manca: chi ha già un `init.lisp` deve
aggiungere a mano le righe `(eval-file ...)` dei moduli nuovi #rapporto("AVV-5").

=== `eval_file`

`eval_file(nome, env)` cerca prima `nome` come percorso. Se non esiste e il nome
non contiene `/`, `\` o `.`, cerca `nome.lisp` in ogni directory di
`lisp-path`, in ordine. Il contenuto viene avvolto in `(progn …)`, letto con
#emph[una sola] chiamata a `Parser::next` e valutato dentro uno scope di
carburante (`begin_command`). Un file mancante produce una riga di log e il
valore `nil`.

#estratto("editor/boot.rs")[
```rust
let content = format!("(progn {})", /* il testo del file, o di nome.lisp in lisp-path */);
let ast = if let Ok(ast) = Parser::new(&content).next() { ast } else {
    return Ok(ELispExp::nil());           // errore di sintassi: nessun messaggio
};
let _command = self.begin_command();      // uno scope di carburante
Ok(ELispExp::proper_list(vec![eval(&ast, env.clone(), self)?]))
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
ogni clone vede lo stesso stato. I suoi campi sono privati, e ognuno è o un
#emph[compartimento] di `managers/` sotto un `RwLock`, o un dato piccolo con un
lock o un atomico proprio.

#ref-table(
  columns: (3.4cm, 3.6cm, 1fr),
  header: ([Campo], [Tipo], [Contenuto]),
  [`buffers`], [`Buffers<B>`], [Tabella nome → `Arc<RwLock<Buffer>>`, buffer corrente, ordine di recente uso.],
  [`windows`], [`Windows`], [Albero delle finestre affiancate, finestre flottanti, focus, prossimo id, trascinamento, ultimo clic.],
  [`modes`], [`Modes<B>`], [Registro dei modi maggiori, keymap globale, completamenti globali, modi per estensione, keymap transitoria, tasti di ripetizione, hook globali.],
  [`commands`], [`Commands<B>`], [Registro dei comandi, pila dei comandi in attesa di argomenti, argomento prefisso, ultimo comando e sua forma.],
  [`kill_yank`], [`KillYank`], [Kill ring, ultimo yank, flag «il comando precedente ha ucciso/incollato».],
  [`macros`], [`Macros`], [Macro in registrazione, ultima macro, contatore.],
  [`history`], [`History`], [Storia dei prompt per chiave (100 voci per chiave predefinite).],
  [`log`], [`Log`], [Righe diagnostiche e file a cui sono copiate.],
  [`runtime`], [`Runtime`], [Colonna obiettivo, isearch e sostituzione in corso, `FuelMeter`, pila delle chiamate per i backtrace, tema, generazioni dei worker.],
  [`echo_message`], [`RwLock<EchoMessage>`], [Testo dell'area messaggi con l'istante in cui è stato scritto.],
  [`pending_keys`], [`RwLock<Vec<KeyEvent>>`], [Tasti di una sequenza non ancora completa.],
  [`owed`], [`RwLock<Vec<OwedCallback>>`], [Callback chieste dai lavori di sfondo e non ancora eseguite.],
  [`current_results`], [`RwLock<Option<String>>`], [Il buffer di risultati che `next-error` percorre.],
  [`worker_mailbox`], [`Sender<WorkerMessage>`], [Canale verso lo scheduler.],
  [`running`], [`AtomicBool`], [Falso dopo `quit`.],
  [`shell_commands`, `background_work`], [`AtomicUsize`], [Quanti comandi di shell e lavori di sfondo stanno cambiando lo schermo.],
  [`command_errors`], [`AtomicUsize`], [Quanti comandi sono falliti: la riproduzione di una macro si ferma al primo.],
)

`EditorState` implementa `Debug` e `PartialEq` solo perché `LispContext` li
richiede; nessuno dei due è pensato per essere usato #rapporto("DOC-2").

== Le regole dei compartimenti

Il commento in testa a `managers/mod.rs` le enuncia; il codice le rispetta.

#ref-table(
  columns: (5.0cm, 1fr),
  header: ([Regola], [Perché]),
  [Un compartimento non raggiunge un altro compartimento.], [Riceve i valori che gli servono e restituisce risposte (`Windows::scroll_focused` riceve un numero di righe, non un buffer). Così solo la facciata tiene due lock insieme.],
  [Non consegna guardie.], [Si accede allo stato solo con una closure (`windows(|w| …)`), e il lock non sopravvive alla domanda.],
  [Non valuta mai.], [Nessun compartimento riceve un `Env`. Può conservare valori Lisp (il corpo di un hook), mai eseguirli: Lisp può rientrare nell'editor da qualunque primitiva.],
  [Risponde con un verdetto, non con un effetto.], [Quando un'operazione ha conseguenze altrove (un buffer da ripuntare, una finestra da rifocalizzare) restituisce un enum come `BufferRemoved` o `WindowRemoved`, e la facciata agisce.],
  [Non fa nulla di lento con il lock preso.], [Niente disco, rete o attese. `Log::record` restituisce il file e la scrittura avviene dopo aver rilasciato il lock.],
)

La regola che nasce da tutte queste, e che vale anche fuori dai compartimenti,
è una sola: #emph[mai tenere un lock attraverso una chiamata all'interprete].
`run_hook` copia la lista degli hook e rilascia `modes` prima di valutarli;
`command_specs` restituisce dati posseduti perché `call-interactively` valuta
Lisp subito dopo. Un `RwLock` non è rientrante: un hook che chiama `add-hook`
mentre lo stesso thread tiene il lock di lettura del registro si blocca per
sempre, ogni volta.

== L'ordine dei lock

Quando un'operazione deve tenere più lock insieme, li prende in quest'ordine.

#canvas(height: 3.4cm)[
  #node(1.4cm, 1.0cm, 2.4cm, 0.8cm, [ambiente Lisp\ (`Env`)], fill: paper)
  #node(4.4cm, 1.0cm, 2.6cm, 0.8cm, [`pending_keys`])
  #node(7.4cm, 1.0cm, 2.4cm, 0.8cm, [`echo_message`])
  #node(10.2cm, 1.0cm, 2.2cm, 0.8cm, [`windows`])
  #node(12.8cm, 1.0cm, 2.0cm, 0.8cm, [`buffers`])
  #node(15.0cm, 1.0cm, 1.9cm, 0.8cm, [un `Buffer`])
  #carrow((2.6cm, 1.0cm), (3.1cm, 1.0cm))
  #carrow((5.7cm, 1.0cm), (6.2cm, 1.0cm))
  #carrow((8.6cm, 1.0cm), (9.1cm, 1.0cm))
  #carrow((11.3cm, 1.0cm), (11.8cm, 1.0cm))
  #carrow((13.8cm, 1.0cm), (14.05cm, 1.0cm))
  #node(15.0cm, 2.6cm, 1.9cm, 0.7cm, [`modes`])
  #carrow((15.0cm, 1.4cm), (15.0cm, 2.25cm))
  #note(0.2cm, 2.15cm)[le variabili si leggono #emph[prima] di ogni altro lock: `snapshot` legge le impostazioni,\ poi l'area messaggi, poi `windows` e `buffers` insieme; la risoluzione di un tasto tiene `pending_keys`,\ legge il modo del buffer corrente e solo dopo prende `modes`.]
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
cambiano quale buffer è corrente passano tutte da `make_current`, che rifiuta
un nome assente dalla tabella; questa invariante esiste perché una finestra
rimasta a mostrare un buffer cancellato aveva già fatto crollare l'editor.

== L'editor come contesto dell'interprete

`EditorState<B>` è il `T: LispContext` dell'interprete:

#ref-table(
  columns: (5.4cm, 1fr),
  header: ([Metodo del trait], [Implementazione]),
  [`consume_fuel(n)`], [`FuelMeter` di `Runtime` (clonato fuori dal lock); `Exhausted` diventa `OutOfFuel`.],
  [`log_diagnostic(msg)`], [`Log::record` sotto lock, poi la scrittura sul file senza lock.],
  [`begin_unwind()`], [Alza il carburante ad almeno 100.000 passi #rapporto("INT-1").],
  [`begin_thread_evaluation()`], [`arm_thread()`: il nuovo thread riceve il budget configurato.],
  [`push_call_frame` / `pop_call_frame` / `call_frame_depth` / `truncate_call_frames`], [La pila `call_stack: Vec<String>` di `Runtime`, che `report_error` legge come backtrace.],
)

Lo scope di carburante dell'editor è `begin_command()`: lo aprono
`run_command_form`, ogni hook, `report_error`, ogni callback dovuta, ogni
funzione di `after-resize-hook` ed `eval_file`. Solo l'apertura più esterna
ricarica il budget (10.000.000 di passi), quindi un `eval-file` dentro un
comando consuma il budget del comando. Gli hook, invece, sono valutati dopo che
lo scope del comando si è chiuso: ognuno riparte con il budget pieno.

// ===========================================================================
= Dal tasto al comando <tasto>
// ===========================================================================

== Il vocabolario dei tasti

Un `KeyEvent` è un `KeyCode` (`Char(c)`, `Backspace`, `Enter`, `Esc`, `Tab`, le
quattro frecce) più i modificatori `ctrl`, `alt`, `shift`. Nelle associazioni i
tasti si scrivono come in Emacs: `C-x`, `M-x`, `C-M-f`, `S-<tab>`, con i nomi
`<ret>`, `<esc>`, `tab`, `<backtab>`, `<backspace>`, `<space>`, `<left>`,
`<right>`, `<up>`, `<down>`. Una sequenza è una stringa di tasti separati da
spazi: `"C-x C-f"`. `parse_key_sequence` (in `primitives/`) legge queste
stringhe; `describe_keys` fa il contrario.

La keymap globale scritta in Rust contiene pochissimo: `M-x`
(`command-execute-prompt`), le frecce, Invio e i caratteri stampabili da `' '` a
`'~'`, legati a `(self-insert "c")`. Tutto il resto --- `C-x C-f`, `C-s`,
`C-k`, … --- viene da `common-keymaps.lisp` e dagli altri moduli.

Solo i caratteri ASCII stampabili sono legati a `self-insert`, e non esiste
un'associazione di ripiego per gli altri: un carattere come «è» arriva alla
risoluzione senza un comando e produce `è is undefined` #rapporto("CMD-1").

== `handle_key_event`

#canvas(height: 11.6cm)[
  #node(4.6cm, 0.45cm, 6.4cm, 0.7cm, [`KeyEvent` dal front-end], fill: accent.lighten(85%))
  #carrow((4.6cm, 0.8cm), (4.6cm, 1.2cm))
  #node(4.6cm, 1.6cm, 6.4cm, 0.8cm, [`*key-capture-function*` non nil?])
  #carrow((7.8cm, 1.6cm), (9.6cm, 1.6cm), label: [sì], dy: -7pt)
  #node(12.9cm, 1.6cm, 6.4cm, 0.8cm, [`capture_key_sequence`: stessa risoluzione,\ poi `(funcall WATCHER KEYS TARGET SOURCE)`])
  #carrow((4.6cm, 2.0cm), (4.6cm, 2.45cm))
  #node(4.6cm, 2.85cm, 6.4cm, 0.8cm, [`read_prefix_argument`: `C-u`, cifre, `-`?])
  #carrow((7.8cm, 2.85cm), (9.6cm, 2.85cm), label: [preso], dy: -7pt)
  #node(12.9cm, 2.85cm, 6.4cm, 0.7cm, [registrato per le macro; fine])
  #carrow((4.6cm, 3.25cm), (4.6cm, 3.7cm))
  #node(4.6cm, 4.15cm, 6.4cm, 0.9cm, [`resolve_key_sequence`:\ transitoria → modo → globale])
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
    if self.read_prefix_argument(&event) {                  // C-u, cifre, -
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

`C-u` e le cifre non arrivano mai alle keymap: `Commands::read_prefix_argument`
li prende per primo. Lo stato è la coppia `prefix_arg: Option<PrefixArg>` e
`reading_prefix: bool`.

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
`modes`, la sequenza viene cercata in tre keymap, in ordine:

+ la #emph[keymap transitoria], se installata;
+ la keymap del modo maggiore del buffer corrente;
+ la keymap globale.

#estratto("editor/keys.rs")[
```rust
let mut pending = self.pending_keys.write().unwrap();
pending.push(event);
let current_mode = self.with_current_buffer(|buf| buf.current_mode.clone());
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
    // prima il modo, poi la keymap globale
    let hit = modes.binding_below_transient(Some(&current_mode), &pending);
    let prefix = modes.is_prefix_below_transient(Some(&current_mode), &pending);
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
  [`Refuse`], [Il tasto è rifiutato in silenzio; la mappa resta. Il messaggio della mappa occupa la riga dei messaggi.], [Una domanda (`save-some-buffers`, `query-replace`): si risponde solo con i suoi tasti.],
  [`Pass`], [Il tasto prosegue verso le altre keymap; la mappa resta.], [La striscia dei completamenti: si può continuare a scrivere.],
  [`Release`], [Il tasto prosegue; la mappa viene tolta.], [La ripetizione (`C-x o o o`): un'offerta che si può ignorare.],
)

Dopo ogni comando, `install_repeat_keymap` controlla se il comando ha un
#emph[tasto di ripetizione] dichiarato (`define-repeat-key`) e in tal caso
installa una mappa `Release` che lo associa al comando stesso.

== `run_command_form`: l'esecuzione di un comando

Tutto ciò che il tasto, l'incolla, la cattura e la riproduzione di macro fanno
eseguire passa da qui.

#canvas(height: 10.4cm)[
  #lane(1.5cm, 0cm, 10.3cm, [`run_command_form`], w: 2.9cm)
  #lane(5.3cm, 0cm, 10.3cm, [undo / kill])
  #lane(8.6cm, 0cm, 10.3cm, [`eval`])
  #lane(11.7cm, 0cm, 10.3cm, [`Commands`])
  #lane(14.6cm, 0cm, 10.3cm, [`Modes`])

  #msg(1.5cm, 5.3cm, 1.2cm, [`undo::begin_command(self-insert?)`])
  #msg(1.5cm, 5.3cm, 1.7cm, [`roll_over_command_flags`])
  #seq-note(1.65cm, 1.9cm)[forma senza argomenti `(nome)` o `nome`\ → `(call-interactively "nome")`]
  #msg(1.5cm, 8.6cm, 3.1cm, [`begin_command()` + `eval(ast)`])
  #msg(8.6cm, 1.5cm, 3.6cm, [`Ok` / `Err`], dash: "dashed")
  #msg(1.5cm, 11.7cm, 4.1cm, [`clear_prefix_argument`])
  #seq-note(1.65cm, 4.3cm)[`Err` → `report_error` → `(report-error MSG FRAMES)`; fine]
  #msg(1.5cm, 14.6cm, 5.3cm, [`hooks_for(modo, "post-self-insert-hook")`])
  #msg(1.5cm, 8.6cm, 5.8cm, [ogni hook, con il suo scope], dash: none)
  #msg(1.5cm, 14.6cm, 6.4cm, [`hooks_for(modo, "post-command-hook")`])
  #msg(1.5cm, 8.6cm, 6.9cm, [ogni hook, con il suo scope])
  #msg(1.5cm, 11.7cm, 7.5cm, [`set_last_form(ast)` (non per `repeat`)])
  #msg(1.5cm, 14.6cm, 8.1cm, [`install_repeat_keymap`])
  #msg(1.5cm, 11.7cm, 8.7cm, [`set_last_command(nome)`])
  #seq-note(1.65cm, 9.0cm)[`post-self-insert-hook` solo se il comando è `self-insert`]
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
    let outcome = { let _command = self.begin_command(); eval(&ast, env.clone(), self) };
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
+ #emph[Esecuzione.] `eval` dentro uno scope di carburante.
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

== La cattura di una sequenza

`read-key-sequence` arma `*key-capture-function*`. Il tasto successivo segue la
stessa risoluzione di un tasto normale --- la sequenza a metà si vede come
sempre --- ma al posto di eseguire il comando viene chiamata la funzione con la
sequenza, la forma che sarebbe stata eseguita (citata) e il nome della keymap
che la fornisce, o `"prefix-argument"` per `C-u`. È così che `describe-key` e
`zap-to-char` leggono un tasto: non esiste in rsedit un «leggi un tasto» che
blocchi, perché i tasti arrivano uno per giro del ciclo degli eventi.

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
comunque legare a un tasto, e viene chiamata senza argomenti. Le specifiche
sono analizzate una sola volta, alla registrazione: un codice sbagliato è un
errore all'avvio, non la prima volta che il comando gira.

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
inizia, non quando l'ultimo argomento arriva. È ciò che l'utente intendeva
premendo `C-u 4 M-x foo`, e nel frattempo il prompt ha cambiato buffer e
l'argomento prefisso è già stato cancellato.

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
  [`syntax`, `scan`], [Cache della colorazione e della scansione delle espressioni.],
  [`file_stamp`, `stale`], [Timbro (istante di modifica e lunghezza) del file all'ultima lettura o scrittura; flag «cambiato sotto di noi».],
  [`auto_saved_at`], [Versione scritta nell'ultimo salvataggio automatico.],
  [`data`], [`HashMap<String, Box<dyn Any>>`: dati che altre parti appendono al buffer e che muoiono con lui.],
  [`local_keymap`], [Dichiarato ma mai usato #rapporto("DOC-2").],
)

== `BufferTrait` e `GapBuffer`

`BufferTrait` è l'unica cosa che il resto dell'editor può assumere sul testo:
lunghezza, carattere in una posizione, punto in coordinate lineari e in
(riga, colonna), conversioni tra le due, spostamenti del punto, inserimento e
cancellazione di un carattere #emph[al punto], righe, `clear`. Gli
spostamenti e le conversioni lavorano in caratteri, non in byte.

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

== Le due porte

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
  #node(12.3cm, 5.05cm, 7.2cm, 1.4cm, [`changed(buf, at)`:\ `version += 1`;\ `syntax.invalidate_from(riga)`, `truncate`;\ `scan.invalidate_from(riga)`], fill: paper)
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
fn changed<B: BufferTrait>(buf: &mut Buffer<B>, at: usize) {
    buf.version = buf.version.wrapping_add(1);
    let line = buf.text.cursor_1d_to_2d(at.min(buf.text.len())).0;
    buf.syntax.invalidate_from(buf.version, line);
    buf.syntax.truncate(buf.text.line_count());
    buf.scan.invalidate_from(buf.version, line);
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
avviene sotto lo stesso lock di scrittura del buffer).

Tre percorsi oggi scavalcano le porte:

- `adopt_text` (lettura di un file, `revert-buffer`, ricarica dal watcher)
  sostituisce tutto il testo e fa a mano ciò che le porte farebbero: incrementa
  la versione, invalida entrambe le cache dalla riga 0, svuota overlay, testo
  virtuale, mark e storia dell'undo (conservandone il limite), riporta il punto
  alla stessa riga e colonna;
- `set_minibuffer_content`, usata dallo scorrimento dei completamenti, scrive
  direttamente nel testo del minibuffer #rapporto("MIN-3");
- `undo` e `redo`, che applicano l'inverso di un gruppo direttamente sul testo
  con `UndoHistory::undo(text)`: non controllano `read_only`, non incrementano
  la versione, non invalidano le cache e non adattano gli overlay
  #rapporto("BUF-1").

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

== Mark, regione, overlay e testo virtuale

Il mark è una posizione con un flag `active`. La regione esiste solo se il mark
è attivo (`region_bounds` la ordina e la limita alla lunghezza del testo), e
ogni modifica la disattiva, come in Emacs. Gli #emph[overlay] sono intervalli con
una faccia e una categoria (le corrispondenze di una ricerca, una diagnostica,
l'enfasi di una pagina di manuale); sono la prima cosa che deve sopravvivere
alle modifiche, e le porte li adattano con `adjust_for_insert` e
`adjust_for_delete`; un overlay che si riduce a lunghezza zero viene cancellato.
Il #emph[testo virtuale] è ancorato a un punto, non a un intervallo, ed è per
questo una tabella a parte: il layout delle righe lo inserisce nel testo
mostrato (`ui/layout.rs`), e un clic su di esso viene riportato alla posizione
vera del buffer.

== Le cache del testo

#ref-table(
  columns: (2.6cm, 1fr, 1fr),
  header: ([Cache], [Che cosa conserva], [Chi la riempie]),
  [`SyntaxCache`], [Gli intervalli colorati di ogni riga e lo stato del lessico (la pila delle regioni aperte) all'inizio di ogni riga, per versione.], [L'evidenziatore, 500 righe per turno, ogni 40 ms.],
  [`ScanCache`], [Un punto di ripresa ogni 64 righe per la scansione delle espressioni bilanciate, per versione e modo.], [Lo scanner preventivo, 500 righe per turno. `forward-sexp`, l'indentazione e `syntax-ppss` la leggono e partono dal punto di ripresa più vicino invece che dall'inizio.],
)

Entrambe sono invalidate dalla riga modificata in giù, dalla stessa chiamata
(`changed`). Una cache che non corrisponde a versione e modo correnti non viene
usata: il caso peggiore è una scansione dall'inizio, cioè il comportamento che
c'era prima delle cache.

== File su disco

`find-file` legge il file, imposta `file_path` e il timbro (`FileStamp`:
istante di modifica e lunghezza, un solo `stat`), e sceglie il modo secondo le
espressioni regolari di `add-auto-mode`. La lettura usa `read_to_string`, che
rifiuta un file non UTF-8; il motivo va nel registro #rapporto("FIL-3").
Salvare scrive l'intero testo con `std::fs::write` (sul posto, senza file
temporaneo #rapporto("FIL-2")) e aggiorna il timbro, altrimenti ogni
salvataggio sembrerebbe la scrittura di qualcun altro. `save-buffer` scrive
l'esito, riuscito o fallito, nel registro e non nell'area messaggi
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
  [`search.rs`], [`Pattern` (che cosa cercare) con `search_forward`, `search_backward`, `scan`; le sessioni `Isearch` e `Replace`; `Casing` ed `expand_replacement`.], [`feature/isearch.rs`, `primitives/isearch.rs`, `editor/replace.rs`, `primitives/scan.rs`],
  [`kill_ring.rs`], [`KillRing`: un anello di stringhe con la regola di rotazione e di accodamento.], [`managers/kill_yank.rs` (`KillYank`)],
  [`rectangle.rs`], [`Rectangle` e `Span`: l'aritmetica di due angoli e delle righe tra loro.], [`primitives/rectangle.rs`],
  [`results.rs`], [`Entry` e `Results`: una lista di posti e un cursore che la percorre.], [`primitives/results.rs`],
)

Le sessioni (`Isearch`, `Replace`) sono qui per la stessa ragione: contengono
lo #emph[stato] di un ciclo che l'editor non può scrivere come ciclo, perché
non esiste una lettura bloccante di un tasto. Chi le guida tiene la sessione in
`Runtime` tra un tasto e l'altro e chiama un metodo per ogni risposta
dell'utente; la sessione conosce il buffer solo per nome.

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

== `Pattern`: che cosa cercare

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

== La sessione di ricerca incrementale: `Isearch`

La parte di interfaccia è descritta nel @minibuffer: `C-s` apre il prompt in
`isearch-mode` e il `post-command-hook` di quel modo chiama `isearch-update`
dopo ogni comando. Qui interessa lo stato che passa da un tasto all'altro.

#estratto("text/search.rs")[
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

== La sessione di sostituzione: `Replace`

`replace-string`, `replace-regexp` e `query-replace` creano una `Replace` con
`begin_replace` (`editor/replace.rs`). Il campo d'azione è la regione se il mark
è attivo, altrimenti dal punto alla fine del buffer: `origin` e `limit` sono i
due estremi. Poi `seek` cerca la prima corrispondenza e un overlay della
categoria `replace-match` la evidenzia.

#estratto("text/search.rs")[
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
tasto chiude la sostituzione e poi fa ciò che fa di solito. Un'interfaccia
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

== Il kill ring

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

== I rettangoli

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

Le colonne sono caratteri. rsedit non espande le tabulazioni (ne disegna una
per cella), quindi colonna di carattere e colonna sullo schermo coincidono
#rapporto("TUI-3").

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

== Le liste di risultati

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

== La regola

`background/worker.rs` enuncia sette regole per tutto ciò che gira fuori dal
thread dei comandi. In breve: ciò che può durare più di un frame è un lavoro di
sfondo avviato da una sola chiamata; un lavoro non disegna mai; non presume che
l'editor sia rimasto fermo e ricontrolla prima di salvare; il progresso è un
segnale, non un ridisegno; le callback girano sul thread dei comandi, tra un
comando e l'altro; gli errori vanno nel log e il lavoro dichiara comunque di
aver finito; un lavoro con lo stesso nome di uno in corso lo sostituisce.

== Lo scheduler

`BackgroundScheduler::spawn` avvia un thread che riceve `WorkerMessage` dalla
mailbox dell'editor.

#canvas(height: 6.0cm)[
  #node(2.6cm, 0.6cm, 4.8cm, 0.8cm, [`worker_mailbox.send(msg)`], fill: paper)
  #carrow((5.0cm, 0.6cm), (6.9cm, 0.6cm))
  #node(10.6cm, 0.6cm, 7.2cm, 0.8cm, [`recv_timeout(prossima scadenza)` o `recv()`], fill: accent.lighten(85%))
  #carrow((8.5cm, 1.0cm), (6.5cm, 1.9cm))
  #carrow((12.7cm, 1.0cm), (13.4cm, 1.9cm))
  #node(4.6cm, 2.5cm, 6.6cm, 1.2cm, [`RunNow(ImmediateTask)`\ un thread nuovo, subito; nessuno lo attende\ (`ShellTask`, `TreeSearch`)])
  #node(12.8cm, 2.5cm, 6.0cm, 1.2cm, [`Schedule { task, interval }`\ aggiunto alla lista dei lavori])
  #carrow((12.8cm, 3.1cm), (12.8cm, 3.75cm))
  #node(12.8cm, 4.4cm, 6.0cm, 1.3cm, [a ogni risveglio: ogni lavoro scaduto\ esegue `execute(&state) -> bool`;\ `false` lo toglie dalla lista])
  #note(0.3cm, 3.6cm)[Il thread esce quando il canale si chiude. Tutti i lavori a turni\ condividono questo solo thread: un turno che dura troppo\ ritarda la colorazione e ogni altro lavoro.]
]

#estratto("background/mod.rs")[
```rust
std::thread::spawn(move || {
    let mut scheduled_jobs: Vec<Job<B>> = Vec::new();
    loop {
        let now = Instant::now();
        let time_to_next_job = scheduled_jobs.iter()
            .map(|job| job.next_run.saturating_duration_since(now)).min();
        let msg = match time_to_next_job {          // si dorme fino alla prossima scadenza
            Some(timeout) => receiver.recv_timeout(timeout),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(WorkerMessage::RunNow(task)) => { /* thread::spawn: task.execute(&state) */ }
            Ok(WorkerMessage::Schedule { task, interval }) =>
                scheduled_jobs.push(Job {
                    task, interval, next_run: Instant::now() + interval,
                }),
            Err(RecvTimeoutError::Disconnected) => break,   // l'editor è stato distrutto
            Err(RecvTimeoutError::Timeout) => {}
        }
        let current_time = Instant::now();
        let _off_the_command_thread = WorkerTurn::begin();
        scheduled_jobs.retain_mut(|job| {
            if job.next_run > current_time { return true; }
            let keep_running = job.task.execute(&state);  // false: il lavoro è finito
            if keep_running { job.next_run = current_time + job.interval; }
            keep_running
        });
    }
});
```
]

Il thread non ha un ciclo di attesa attiva: dorme in `recv_timeout` fino alla
scadenza più vicina, o in `recv` se non ha lavori. Un messaggio lo sveglia in
anticipo. Ogni turno di ogni lavoro gira sullo stesso thread, uno dopo l'altro,
e nessuno lo protegge da un panic: un turno che va in panic termina il thread
e con lui tutti i lavori a turni #rapporto("BKG-1").

#ref-table(
  columns: (3.5cm, 2.6cm, 1fr),
  header: ([Lavoro], [Tipo, intervallo], [Che cosa fa a ogni turno]),
  [`Highlighter`], [a turni, 40 ms], [Per ogni buffer con colorazione da fare: copia fino a 500 righe sotto lock, rilascia, colora, riprende il lock e salva solo se la versione non è cambiata.],
  [`Prescanner`], [a turni, 40 ms], [Lo stesso per la `ScanCache`: 500 righe, un punto di ripresa ogni 64. Esamina le righe sotto il lock di lettura.],
  [`FileWatcher`], [a turni, 3 s], [Confronta i timbri di al massimo 8 file; ricarica o marca `stale`.],
  [`AutoSaver`], [a turni, 30 s], [Scrive al massimo 4 buffer modificati e cambiati dall'ultima copia.],
  [worker Lisp], [a turni, 40 ms o `INTERVAL`], [`resume` della fiber, con `worker-fuel` passi (200.000 per default).],
  [`background-call`], [a turni o una volta], [Una fiber a turni, o una funzione eseguita una volta sul thread dello scheduler.],
  [`ShellTask`], [thread proprio], [Esegue un processo e accoda l'uscita al buffer.],
  [`TreeSearch`], [thread proprio], [Cerca nei file di una directory e scrive i risultati in un buffer.],
)

== Un turno dell'evidenziatore

L'evidenziatore è il lavoro di sfondo più frequente, e il modello per scriverne
altri. Un turno si divide in tre fasi, e nessuna tiene un lock mentre fa il
lavoro vero:

#estratto("modes/highlighter.rs")[
```rust
pub(crate) fn highlight_one_turn(&self) {
    let Some(turn) = self.next_highlight_turn() else { return };  // 1. sotto lock: copia
    let coloured = turn.run();                                     // 2. senza lock: colora
    self.store_turn(&turn, coloured);                              // 3. sotto lock: salva
}
pub(crate) fn store_turn(
    &self, turn: &Turn, coloured: Vec<(usize, SyntaxState, Vec<SyntaxSpan>)>,
) {
    self.with_buffer_mut(&turn.buffer, |buf| {
        if buf.version != turn.version { return; }   // il testo è cambiato: si butta tutto
        for (line, entering, spans) in coloured {
            if buf.syntax.record(line, &entering, spans).is_err() { break; }
        }
    });
}
```
]

+ #emph[Copia.] `next_highlight_turn` sceglie il primo buffer, partendo da
  quello con il focus, la cui cache è indietro rispetto al testo
  (`syntax.valid_to() < line_count()`) e il cui modo ha una grammatica. Copia
  fino a `LINES_PER_TURN` (500) righe a partire dalla prima non valida, lo stato
  del lessico all'inizio di quella riga, la grammatica (clonata dal registro dei
  modi) e la versione del buffer.
+ #emph[Colora.] `Turn::run` chiama `highlight_line` su ogni riga, passando lo
  stato in uscita da una riga come stato in entrata della successiva. Il costo di
  una riga cresce più che linearmente con la sua lunghezza #rapporto("SIN-2").
+ #emph[Salva.] Solo se la versione del buffer è ancora quella copiata: un
  carattere digitato nel frattempo rende inutile tutto il turno, che viene
  rifatto al giro successivo dalla riga modificata.

Lo stesso schema --- copia sotto lock, lavora senza, ricontrolla la versione
prima di salvare --- regge il `Prescanner` (che però esamina le righe sotto il
lock di lettura, perché la scansione è veloce) e l'`AutoSaver`.

== I comandi di shell

`shell-command-start` avvia `/bin/sh -c COMANDO` (o `cmd /C` su Windows) con
stdin chiuso e stdout e stderr in pipe, crea un buffer `*Shell Output*` (o
`<2>`, `<3>`… se il nome è occupato) e spedisce allo scheduler un
`RunNow(ShellTask)`. Il task gira su un thread suo:

#estratto("primitives/shell.rs")[
```rust
fn execute(mut self: Box<Self>, state: &EditorState<B>) {
    if let Some(stdout) = self.child.stdout.take() {
        for line in BufReader::new(stdout).lines() {           // riga per riga, man mano
            match line {
                Ok(line) => { state.append_to_buffer(&self.buffer, &format!("{line}\n")); }
                Err(why) => { /* "[unreadable output: …]" */ break; }
            }
        }
    }
    if let Some(mut stderr) = self.child.stderr.take() {       // poi stderr, tutto insieme
        let mut text = String::new();
        if stderr.read_to_string(&mut text).is_ok() && !text.is_empty() {
            state.append_to_buffer(&self.buffer, &text);
        }
    }
    let status = /* "--- exited N ---" o "--- killed ---" */;
    state.append_to_buffer(&self.buffer, &status);
    state.finish_shell_command();          // il ridisegno periodico può fermarsi
}
```
]

`append_to_buffer` toglie per un attimo la sola lettura del buffer e scrive con
`insert_text`, quindi l'output passa dalle porte come ogni altra modifica e il
renderer lo vede al primo ridisegno (finché un comando è in corso
`next_redraw_in` chiede un ridisegno ogni 40 ms). Lo stdout è letto per intero
prima di cominciare a leggere lo stderr #rapporto("SHL-1"); la prima riga non
UTF-8 interrompe la lettura #rapporto("SHL-2"); non esiste un modo di fermare il
processo #rapporto("SHL-3"). `shell-command-to-string`, al contrario, è
sincrono: aspetta la fine del processo sul thread dei comandi e restituisce lo
stdout come stringa.

== Worker e lavori in Lisp

`(define-worker NOME FIBER [INTERVALLO])` riprende la fiber a ogni turno; la
fiber è un ciclo che fa `(yield)` dove accetta di essere messa da parte. Ogni
turno ha un suo budget, impostato sul thread dello scheduler con
`set_remaining(worker-fuel)`: il carburante è per thread, quindi un comando non
può accorciare il turno di un worker né un worker spendere quello di un
comando. Un turno che supera il budget termina, viene segnalato una volta e il
worker viene ritirato: un `(while t)` senza `yield` fermerebbe altrimenti la
colorazione per sempre.

`(background-call NOME LAVORO [ON-PROGRESS [ON-DONE [INTERVALLO]]])` avvia un
lavoro e restituisce subito. `ON-PROGRESS` è chiamata con il nome dopo ogni
turno tranne l'ultimo; `ON-DONE` una volta, con il nome e `t` (finito) o `nil`
(errore o arresto). `stop-worker` e `running-workers` valgono per entrambi.

Ogni nome ha una #emph[generazione] in `Runtime`: definire di nuovo lo stesso
nome, o fermarlo, incrementa il numero, e il lavoro vecchio si ritira al
prossimo risveglio confrontando la generazione con cui è nato. Non serve un
canale di cancellazione, e ricaricare un modulo non duplica i suoi worker.

== Le callback dovute

Una callback chiesta da un lavoro di sfondo non gira sul thread del lavoro: va
nella lista `owed` dell'editor, con il nome e la generazione del lavoro.

#canvas(height: 5.8cm)[
  #lane(1.6cm, 0cm, 5.7cm, [lavoro (scheduler)], fill: accent.lighten(85%), w: 3.0cm)
  #lane(5.6cm, 0cm, 5.7cm, [`owed`])
  #lane(9.4cm, 0cm, 5.7cm, [ciclo TUI], fill: accent.lighten(85%))
  #lane(13.3cm, 0cm, 5.7cm, [`run_owed_callbacks`], w: 3.2cm)
  #msg(1.6cm, 5.6cm, 1.2cm, [`owe_callback(nome, gen, f, args)`])
  #msg(9.4cm, 5.6cm, 1.8cm, [`next_redraw_in`: `callbacks_owed()`?])
  #msg(5.6cm, 9.4cm, 2.3cm, [sì → attesa zero], dash: "dashed")
  #msg(9.4cm, 13.3cm, 2.9cm, [scade: `tick(&env)`])
  #msg(13.3cm, 5.6cm, 3.4cm, [`take` della lista intera])
  #seq-note(13.0cm, 3.7cm)[generazione superata → scartata]
  #seq-note(10.2cm, 4.45cm)[altrimenti: `begin_command()` + `call_callable`;\ un errore va nel log]
]

#estratto("editor/background.rs")[
```rust
pub fn run_owed_callbacks(&self, env: &Arc<Env<EditorState<B>>>) -> bool {
    let owed = std::mem::take(&mut *self.owed.write().unwrap());   // la lista intera
    let mut ran = false;
    for callback in owed {
        if !self.runtime(|r| r.worker_is_latest(&callback.name, callback.generation)) {
            continue;                       // il lavoro è stato sostituito o fermato
        }
        ran = true;
        let _command = self.begin_command();                         // budget pieno
        if let Err(error) =
            call_callable(&callback.function, &callback.args, env.clone(), self)
        {
            self.log_diagnostic(/* la callback del lavoro è fallita */);
        }
    }
    ran
}
```
]

La lista viene svuotata con un solo `take` prima di eseguire qualunque
callback: una callback che chiede altre callback le aggiunge a una lista nuova,
che girerà al prossimo `tick`, e il lock non resta mai preso mentre gira Lisp.

È qui che un modo «resta coerente» (regola 5): la callback gira tra un comando e
l'altro, sul thread che possiede l'interfaccia, e può aprire buffer, scrivere
messaggi o cambiare la finestra come farebbe un comando.

== Quando ridisegnare

`next_redraw_in(&env, &frame)` è il minimo tra: la scadenza del messaggio
nell'area messaggi (`echo-message-timeout`, 5 s; `nil` non scade mai);
40 ms se la colorazione è in corso, se c'è un comando di shell attivo o se un
lavoro di sfondo sta cambiando lo schermo; zero se ci sono callback dovute.
`None` significa che nulla è in sospeso e il front-end può bloccarsi
sull'input. `drag_scroll_in()` aggiunge i 60 ms dello scorrimento automatico
mentre si trascina una selezione oltre il bordo di una finestra.



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
un `Removed` (`No`, `Yes`, `Refocus(id)`) che dice alla facciata se e verso
quale finestra spostare il focus.

== `snapshot`: dalla memoria alla fotografia

`snapshot(&env, larghezza, altezza)` costruisce il `FrameSnapshot`. Prende i
lock in un ordine fisso e non fa I/O; il disegno avviene dopo, senza lock.

+ Legge dalle variabili Lisp e dai compartimenti piccoli tutto ciò che serve
  #emph[prima] di toccare finestre e buffer: `echo-message-timeout`, il tema,
  l'input in sospeso (`pending_input`), il messaggio della keymap transitoria,
  `mode-line-format`, `window-separator`, la configurazione del margine
  (`display-line-numbers`), se la colorazione è in corso.
+ Legge l'area messaggi, vuota se il messaggio è scaduto.
+ Con `windows` in scrittura (lo scorrimento può dover seguire il punto) e
  `buffers` in lettura, compone ogni foglia dell'albero nel rettangolo del
  frame meno l'ultima riga, raccogliendo i separatori; poi compone le flottanti.
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
`" %* %b   %m   L%l C%c   %p "`): `%b` nome del buffer, `%f` file (o nome),
`%m` modo, `%l` riga (da 1), `%c` colonna (da 0), `%p` posizione (`All`,
`Top`, `Bot` o percentuale), `%*` `**` se modificato e `--` altrimenti, `%%`.

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

== Il mouse

Con `mouse-mode` attivo la TUI chiede al terminale di riportare il mouse.
`handle_mouse_event` traduce l'evento in un `Hit` (testo di una finestra, riga
di stato, separatore, flottante, niente), e da lì in un comando: clic per
spostare il punto e il focus, doppio clic, trascinamento per selezionare (con
scorrimento automatico ogni 60 ms oltre il bordo), trascinamento di un
separatore o di una riga di stato per ridimensionare, rotella per scorrere la
finestra sotto il puntatore. Risponde `true` solo se ha cambiato qualcosa.

// ===========================================================================
= Macro di tastiera <macro>
// ===========================================================================

Una macro registra #emph[tasti], non comandi: `C-u 5`, un tasto letto da
`read-key-sequence` e perfino un tasto non associato fanno parte di ciò che
l'utente ha premuto. `handle_key_event` chiama `record_keys` nei punti in
cui un tasto viene consumato. La riproduzione rimanda i tasti a
`handle_key_event` dentro un `OneGroup` (un solo gruppo di undo) e si ferma al
primo comando fallito, confrontando `command_errors` prima e dopo ogni tasto.
Una macro che richiama macro può annidarsi fino a 16 livelli (`MAX_DEPTH`).

// ===========================================================================
= Le primitive dell'editor <primitive>
// ===========================================================================

== Il registro

Ogni modulo di `primitives/` ha una funzione `install(into: &Registry)` che
registra le sue primitive con `into.function(nome, puntatore, DOC)` o, per i
comandi, `into.command(nome, puntatore, specifiche, DOC)`.
`install_primitives` chiama tutti i moduli. La macro `primitive!` genera la
firma richiesta da `LispPrimitive`; `args.rs` contiene gli aiuti per leggere gli
argomenti (nomi come simbolo o stringa, numeri, posizioni).

Ogni docstring comincia con la firma, nella forma `(nome ARG):` seguita da una
frase: è ciò che mostrano i comandi di aiuto. `doc_faithfulness_tests`
confronta ciò che l'aiuto racconta con l'editor reale; `primitive_shape_tests`
verifica le forme condivise dalle primitive, come il controllo dell'arità.

#ref-table(
  columns: (2.8cm, 0.8cm, 1fr),
  header: ([Modulo], [N.], [Contenuto]),
  [`edits`], [47], [Inserire, cancellare, uccidere, incollare, undo, movimenti per carattere, parola, riga, paragrafo ed espressione.],
  [`io`], [31], [File: aprire, salvare, rileggere, recuperare, rinominare; percorsi e directory; variabili d'ambiente; `quit` e `save-some-buffers`.],
  [`buffers`], [17], [Creare, cambiare, chiudere buffer; contenuto; sola lettura; dati appesi (`buffer-put`, `buffer-get`).],
  [`region`], [17], [Mark e regione, `keyboard-quit`, kill ring (`kill-region`, `yank`, `yank-pop`, `kill-new`).],
  [`modes`], [15], [`make-mode`, `add-hook`, regole e regioni di sintassi, `add-auto-mode`, tabella sintattica, `syntax-ppss`.],
  [`verbs`], [14], [Maiuscole, spazi, righe vuote, unione e duplicazione di righe, trasposizioni, `zap-to-char`.],
  [`macros`], [12], [Registrazione e riproduzione delle macro, contatore.],
  [`minibuffer`], [12], [`minibuffer-read`, conferma, annullamento, completamento, storia.],
  [`replace`], [12], [Sostituzione e `query-replace`.],
  [`windows`], [12], [Dividere, chiudere, scegliere, scorrere finestre; `display-buffer-at-bottom`.],
  [`general`], [11], [`eval-file`, `define-key`, `define-repeat-key`, `repeat`, `log`, `backtrace`, `regexp-opt`, keymap transitorie.],
  [`results`], [11], [Liste di posizioni (`occur`, compilazione) e `next-error`.],
  [`commands`], [10], [`call-interactively`, `M-x`, `register-command` e il registro dei comandi.],
  [`isearch`], [10], [Ricerca incrementale.],
  [`rectangle`], [10], [Operazioni sui rettangoli.],
  [`completion`], [8], [`completion-at-point`, le sue fonti, `fuzzy-filter`.],
  [`match_data`], [7], [`string-match` e i dati della corrispondenza.],
  [`comments`], [6], [Commentare e decommentare.],
  [`help`], [6], [Ciò che serve all'aiuto: `key-binding`, `where-is`, `keymap-bindings`, `read-key-sequence`. I comandi `describe-*` sono in `help.lisp`.],
  [`mouse`], [6], [Comandi del mouse.],
  [`overlays`], [5], [Overlay.],
  [`theme`], [5], [Facce e stili.],
  [`ask`], [4], [Domande sì/no con keymap transitoria.],
  [`shell`], [4], [Comandi di shell.],
  [`virtual_text`], [4], [Testo virtuale.],
  [`workers`], [4], [`define-worker`, `background-call`, `stop-worker`, `running-workers`.],
  [`ui`], [3], [`recenter` e finestre flottanti.],
  [`scan`], [1], [`scan-buffer`: tutte le corrispondenze di un modello in un buffer, come voci di una lista di risultati.],
)

Il totale è 304, a cui si aggiungono le 86 primitive di base dell'interprete.
L'elenco completo dei nomi è nell'appendice (@elenco-primitive).

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
l'altro, e una callback `ON-DONE` per il risultato. In Rust: un
`ScheduledTask` se il lavoro si fa a turni limitati (copia sotto lock, lavora
senza, ricontrolla la versione prima di salvare), un `ImmediateTask` se può
bloccarsi; spediscilo con `send_to_worker`. In entrambi i casi l'effetto
sull'interfaccia passa da una callback dovuta.

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
/ Callback dovuta: una chiamata chiesta da un lavoro di sfondo ed eseguita dal thread dei comandi.
/ Generazione: il numero che distingue l'esecuzione viva di un lavoro con nome dalle precedenti.
/ Epoca: il timbro del comando corrente, con cui l'undo raggruppa le modifiche.

== Mantenere aggiornato questo manuale

- Nuovo campo di `EditorState` o nuovo compartimento → @stato (tabelle e ordine
  dei lock).
- Cambiamento del percorso di un tasto → @tasto; di `call-interactively` →
  @comandi.
- Nuova primitiva o nuovo modulo di `primitives/` → @primitive e l'elenco in
  appendice (lo script che lo genera cerca `into.function(` e `into.command(`).
- Nuovo lavoro di sfondo → la tabella di @sfondo.
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
  [`buffers` (17)], [`buffer-put`, `buffer-get`, `buffer-substring`, `switch-to-buffer`, `current-buffer`, `buffer-create`, `close-buffer`, `kill-buffer`\*, `kill-buffer-without-saving`\*, `buffer-string`, `clear-buffer`, `with-current-buffer`, `set-buffer-read-only`, `buffer-read-only-p`, `buffer-modified-p`, `buffer-file-name`, `major-mode`],
  [`commands` (10)], [`register-command`, `commandp`, `command-args`, `all-commands`, `execute-extended-command`, `command-completions`, `command-execute-prompt`\*, `all-buffer-names`, `call-interactively`, `pending-command`],
  [`comments` (6)], [`comment-dwim`\*, `comment-line`\*, `comment-indent`\*, `comment-region`\*, `uncomment-region`\*, `comment-syntax`],
  [`completion` (8)], [`completion-at-point`\*, `fuzzy-filter`, `completion-at-point-choose`, `add-completion-function`, `set-completion-functions`, `completion-functions`, `buffer-words`, `bounds-of-thing-at-point`],
  [`edits` (47)], [`delete-region`, `self-insert`, `insert`, `insert-pasted-text`\*, `insert-newline`\*, `delete-backward-char`\*, `backward-char`\*, `forward-char`\*, `previous-line`\*, `delete-char`\*, `kill-line`\*, `kill-whole-line`\*, `kill-word`\*, `backward-kill-word`\*, `kill-paragraph`\*, `backward-kill-paragraph`\*, `undo`\*, `redo`\*, `undo-boundary`\*, `set-undo-limit`\*, `beginning-of-line`\*, `end-of-line`\*, `forward-word`\*, `backward-word`\*, `forward-sexp`\*, `backward-sexp`\*, `kill-sexp`\*, `backward-kill-sexp`\*, `up-list`\*, `backward-up-list`\*, `down-list`\*, `current-column`, `current-indentation`, `previous-indentation`, `indent-line-to`, `forward-paragraph`\*, `backward-paragraph`\*, `beginning-of-buffer`\*, `end-of-buffer`\*, `goto-line`\*, `next-line`\*, `point`, `point-min`, `point-max`, `goto-char`, `line-number-at-point`, `current-line`],
  [`general` (11)], [`eval-file`, `define-key`, `define-repeat-key`, `repeat`\*, `log`, `regexp-opt`, `all-logs`, `backtrace`, `set-echo-message`, `set-transient-keymap`, `clear-transient-keymap`],
  [`help` (6)], [`key-binding`, `keys-with-prefix`, `keymap-bindings`, `where-is`, `command-specs`, `read-key-sequence`],
  [`io` (31)], [`quit`\*, `quit-without-saving`\*, `save-some-buffers`\*, `save-some-buffers--answer`, `quit--confirm`, `find-file`\*, `data-directory`, `getenv`, `setenv`, `path-separator`, `list-dir`, `directory-files-recursive`, `read-file-to-string`, `match-list`, `expand-file-name`, `file-name-as-directory`, `directory-file-name`, `file-name-directory`, `file-name-nondirectory`, `save-buffer`\*, `save-buffer--overwrite`, `recover-file`\*, `recover-file--adopt`, `revert-buffer`\*, `revert-buffer--reread`, `write-file`\*, `file-exists-p`, `file-directory-p`, `directory-entry-count`, `delete-file`, `rename-file`],
  [`isearch` (10)], [`isearch-forward`\*, `isearch-backward`\*, `isearch-forward-regexp`\*, `isearch-backward-regexp`\*, `isearch-update`, `isearch-exit`, `isearch-abort`, `isearch-repeat-forward`, `isearch-repeat-backward`, `isearch-active-p`],
  [`macros` (12)], [`kmacro-start-macro`\*, `kmacro-end-macro`\*, `kmacro-cancel-macro`\*, `kmacro-call-macro`\*, `kmacro-end-and-call-macro`\*, `kmacro-insert-counter`\*, `kmacro-set-counter`\*, `kmacro-counter`, `kmacro-recording-p`, `kbd-macro-keys`, `define-kbd-macro`, `kmacro-call-keys`],
  [`match_data` (7)], [`string-match`, `string-match-p`, `match-beginning`, `match-end`, `match-string`, `match-data`, `set-match-data`],
  [`minibuffer` (12)], [`minibuffer-cleanup`, `minibuffer-confirm`, `minibuffer-cancel`, `minibuffer-complete`, `minibuffer-choose-completion`, `default-minibuffer-prompt`, `minibuffer-read`, `completion-invalidate`, `minibuffer-history`, `clear-minibuffer-history`, `history-previous`\*, `history-next`\*],
  [`modes` (15)], [`make-mode`, `add-hook`, `add-syntax-rule`, `add-syntax-region`, `add-auto-mode`, `syntax-ppss`, `balance-point`, `bounds-of-enclosing-list`, `set-syntax-pairs`, `set-string-syntax`, `set-char-quote`, `set-syntax-entry`, `set-comment-syntax`, `syntax-class`, `matching-delimiter`],
  [`mouse` (6)], [`mouse-set-point`\*, `mouse-start-selection`\*, `mouse-drag-to`\*, `mouse-resize`\*, `mouse-mode-toggle`\*, `mouse-scroll`\*],
  [`overlays` (5)], [`make-overlay`, `delete-overlay`, `remove-overlays`, `overlays-at`, `overlay-face`],
  [`rectangle` (10)], [`rectangle-mark-mode`\*, `kill-rectangle`\*, `delete-rectangle`\*, `copy-rectangle-as-kill`\*, `yank-rectangle`\*, `open-rectangle`\*, `clear-rectangle`\*, `string-rectangle`\*, `rectangle-bounds`, `killed-rectangle`],
  [`region` (17)], [`set-mark`\*, `keyboard-quit`\*, `deactivate-mark`\*, `exchange-point-and-mark`\*, `mark-whole-buffer`\*, `kill-region`\*, `kill-ring-save`\*, `yank`\*, `yank-pop`\*, `set-kill-ring-max`\*, `mark`, `use-region-p`, `region-beginning`, `region-end`, `kill-new`, `current-kill`, `kill-ring-length`],
  [`replace` (12)], [`query-replace`\*, `query-replace-regexp`\*, `replace-string`\*, `replace-regexp`\*, `replace-this`, `replace-skip`, `replace-rest`, `replace-back`, `replace-done`, `replace-abandon`, `replace-match`, `replace-count`],
  [`results` (11)], [`occur--scan`, `grep--scan`, `next-error`\*, `previous-error`\*, `results-count`, `results-entry`, `results-state`, `results-visit`, `results-buffer`, `results-select`\*, `results-put`],
  [`scan` (1)], [`scan-buffer`],
  [`shell` (4)], [`shell-command-start`\*, `shell-command-to-string`, `parse-overstrike`, `shell-command-running-p`],
  [`theme` (5)], [`set-face`\*, `face-style`, `reset-faces`, `list-faces`, `list-colors`],
  [`ui` (3)], [`recenter`\*, `make-floating-window`, `close-floating-window`],
  [`verbs` (14)], [`upcase-word`\*, `downcase-word`\*, `capitalize-word`\*, `delete-horizontal-space`\*, `just-one-space`\*, `delete-blank-lines`\*, `back-to-indentation`\*, `join-line`\*, `duplicate-line`\*, `transpose-lines`\*, `transpose-chars`\*, `transpose-words`\*, `zap-to-char`\*, `zap-to-char--do`],
  [`virtual_text` (4)], [`make-virtual-text`, `clear-virtual-text`, `delete-virtual-text`, `virtual-text-at`],
  [`windows` (12)], [`split-window-below`\*, `split-window-right`\*, `delete-window`\*, `delete-other-windows`\*, `other-window`\*, `selected-window`, `select-window`, `count-windows`, `scroll-up-command`\*, `scroll-down-command`\*, `display-buffer-at-bottom`, `window-buffer`],
  [`workers` (4)], [`define-worker`, `stop-worker`, `running-workers`, `background-call`],
)
