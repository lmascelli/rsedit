// La documentazione di rsedit: dove sta ogni informazione, come si scrive,
// come si controlla, e il piano per arrivarci.
//
// Il capitolo «Le regole» è permanente: vale per ogni modifica futura.
// «L'inventario» fotografa la revisione 58d0b0d e resta, come termine di
// confronto, fino all'ultimo passo; «Il piano» si accorcia a ogni passo.

#import "style.typ": *

#show: preamble.with(
  title: "La documentazione di rsedit",
  subtitle: [Regole, inventario e piano del riordino --- passo 0],
)

#set text(lang: "it")
#set heading(supplement: [§])
#show table: set par(justify: false)
#set figure(kind: image)
#show raw.where(block: true): it => block(breakable: false, it)
#show list: set par(justify: false)
#show enum: set par(justify: false)
#show figure.caption: set align(start)

// Le figure disegnano il codice in linea piccolo e senza sfondo.
#let canvas = canvas.with(code-size: 7.4pt)

// Il riempimento dei nodi che sono fonti per l'utente.
#let user-fill = accent.lighten(85%)

#outline(depth: 2, indent: auto)

// ===========================================================================
= In breve
// ===========================================================================

Oggi la stessa informazione su rsedit è scritta in più posti: nei commenti del
codice, nelle docstring che `C-h f` mostra, nei manpage di `man/` e nei
documenti typst di `doc/`. Le copie non si aggiornano insieme e divergono.
L'elenco dei moduli caricati all'avvio, per esempio, compare in sei posti,
in cinque versioni diverse (@copie). Nei commenti ci sono 145 sezioni
`# Why` e circa 80 righe che raccontano com'era il codice prima. Il manuale
per sviluppatori descrive ancora lo scheduler che le patch 1--9 hanno
sostituito.

Questo documento stabilisce dove sta ogni informazione, come si scrive e come
si controlla che non diverga. Poi misura lo stato attuale e propone i passi
per arrivarci.

#caution[
  *Stato: approvato; i passi 1--5 sono fatti.* Le misure dell'inventario si
  riferiscono alla revisione `58d0b0d`. Le decisioni prese sono raccolte nel
  @domande.
]

#key[
  *Le regole, in sei righe.*
  + *Ogni fatto ha una casa sola.* Altrove si rimanda, non si copia.
  + *Chi usa l'editor legge i manpage*, completi e con esempi pratici. La
    docstring è il promemoria breve che `C-h f` mostra, e porta alla pagina.
  + *Chi sviluppa legge il manuale typst*: architettura, thread, lock e
    decisioni, con diagrammi. Il rustdoc è il contratto breve di ogni elemento.
  + *Un commento dice solo ciò che il codice non può dire*: un'invariante, un
    ordine di lock, un perché non ovvio, un TODO.
  + *Nel codice c'è solo il presente.* La storia sta nei commit, i difetti nel
    rapporto sui problemi.
  + *Una copia è ammessa solo se un test la confronta con la sua fonte.*
]

Il @regole è la parte permanente: vale per ogni modifica futura. Il
@inventario fotografa lo stato di partenza e resta, come termine di
confronto, fino all'ultimo passo. Il @piano propone i passi del riordino e si
accorcia man mano che vengono fatti.

// ===========================================================================
= Le regole <regole>
// ===========================================================================

== Chi legge che cosa

#ref-table(
  columns: (3.4cm, 4.4cm, 1fr),
  header: ([Chi], [Dove guarda], [Che cosa cerca]),
  [Chi usa l'editor], [i manpage: `C-h m`, `M-x manpage`], [come si fa una cosa, con esempi da copiare; i tasti; i limiti],
  [Chi configura l'editor o scrive moduli Lisp], [i manpage e le docstring: `C-h f`, `C-h v`], [in più, la firma esatta di una funzione e il nome di una variabile],
  [Chi sviluppa il Rust], [il manuale in `doc/`, il rustdoc, i commenti], [come è fatto il codice, perché, dove intervenire],
  [Chi indaga un difetto], [`doc/rapporto-problemi.typ`, `git log`], [che cosa è già noto, quando un comportamento è cambiato],
)

== Dove sta ogni informazione <mappa>

#ref-table(
  columns: (5.1cm, 4.3cm, 1fr),
  header: ([Informazione], [Casa], [Altrove]),
  [Che cosa fa una funzione, un comando o una variabile: firma, argomenti, valore], [la sua docstring], [la SYNOPSIS della sua pagina la elenca; `C-h f` aggiunge il rimando alla pagina],
  [Come si usa una funzionalità: concetti, combinazioni, esempi, limiti], [il manpage del suo argomento], [nessuna copia nelle docstring],
  [I tasti], [i keymap dei moduli Lisp], [le sezioni KEYS delle pagine, controllate da un test; `C-h b` li calcola],
  [I moduli caricati e il loro ordine], [`DEFAULT_INIT_LISP` in `editor/boot.rs`], [la sezione MODULES di `rsedit.txt`, controllata; `build.rs` e i test non ne tengono una propria],
  [L'elenco delle pagine], [la directory `man/`], [l'indice di `guide.txt`, controllato; i test leggono la directory],
  [Dove l'editor tiene i suoi file: configurazione, cache, log], [il codice, `paths.rs`], [la pagina `configuration`, per l'utente],
  [La struttura: moduli, tipi principali, thread, lock, flussi], [il manuale per sviluppatori, `doc/architettura-editor.typ`], [la documentazione di modulo: poche righe e il rimando],
  [Perché il codice è fatto così], [le decisioni del capitolo del manuale], [nel codice, al più una riga e il rimando],
  [Invarianti, ordine dei lock, `SAFETY`, il thread su cui gira una funzione], [il commento o il rustdoc accanto al codice], [il manuale riassume l'ordine dei lock dell'intero editor],
  [Il contratto di un elemento Rust: che cosa fa, errori, panic], [il suo rustdoc], [---],
  [La storia: che cosa c'era prima e perché è cambiato], [il messaggio di commit], [ciò che ne resta valido è una decisione del manuale],
  [I difetti noti e il lavoro rimandato], [una scheda di `rapporto-problemi.typ`], [`TODO(ID)` nel codice, finché la scheda è aperta],
  [Il comportamento atteso, caso per caso], [i test: il nome dice il comportamento], [---],
  [Come si misurano le prestazioni], [`PERFORMANCE.md`], [---],
  [Che cos'è rsedit, come si compila, quali documenti ci sono e per chi], [`README.md`], [per tutto il resto rimanda ai manpage e ai documenti],
)

`todo.org` è la lista personale dell'autore e resta fuori da queste regole.

#figure(
  canvas(height: 6.3cm)[
    #node(2.0cm, 1.3cm, 3.4cm, 1.3cm, [*manpage* \ `man/*.txt` \ l'uso completo, gli esempi], fill: user-fill)
    #node(6.1cm, 1.3cm, 3.4cm, 1.3cm, [*docstring* \ `C-h f`, `C-h v` \ firma ed esempio breve], fill: user-fill)
    #node(10.2cm, 1.3cm, 3.4cm, 1.3cm, [*rustdoc* \ `///`, `//!` \ il contratto di ogni elemento])
    #node(14.2cm, 1.3cm, 3.4cm, 1.3cm, [*git* \ i messaggi di commit \ la storia], fill: paper)

    #node(2.0cm, 4.3cm, 3.4cm, 1.3cm, [*test* \ il nome dice il comportamento], fill: paper)
    #node(6.1cm, 4.3cm, 3.4cm, 1.3cm, [*commenti* \ invarianti, lock, \ `SAFETY`, `TODO`])
    #node(10.2cm, 4.3cm, 3.4cm, 1.3cm, [*manuale typst* \ architettura, thread, \ decisioni, diagrammi])
    #node(14.2cm, 4.3cm, 3.4cm, 1.3cm, [*rapporto* \ i difetti noti, \ ciascuno con un ID], fill: paper)

    #carrow((4.4cm, 1.3cm), (3.7cm, 1.3cm), label: [1], dy: -8pt)
    #carrow((10.2cm, 1.95cm), (10.2cm, 3.65cm), label: [2], dx: 8pt)
    #carrow((7.8cm, 4.3cm), (8.5cm, 4.3cm), label: [3], dy: -8pt)
    #segment((6.1cm, 4.95cm), (6.1cm, 5.75cm))
    #segment((6.1cm, 5.75cm), (14.2cm, 5.75cm))
    #arrow((14.2cm, 5.75cm), (14.2cm, 4.95cm))
    #note(10.0cm, 5.45cm)[4]
    #carrow((14.2cm, 1.95cm), (14.2cm, 3.65cm), label: [5], dx: 8pt)
    #carrow((2.0cm, 3.65cm), (2.0cm, 1.95cm), label: [6], dx: 8pt, dash: "dashed", colour: warm)
    #carrow((2.9cm, 3.65cm), (5.2cm, 1.95cm), label: [7], dx: 6pt, dy: 4pt, dash: "dashed", colour: warm)
  ],
  caption: [
    Le fonti e i loro rimandi. Frecce piene: «rimanda a»; tratteggiate:
    «controlla». In azzurro ciò che legge l'utente. (1) `C-h f` aggiunge
    «Manual:» con la pagina che elenca il nome nella sua SYNOPSIS. (2) La
    documentazione di modulo rimanda al capitolo, con un'etichetta. (3) Un
    perché lungo diventa una decisione del manuale. (4) `TODO(ID)` rimanda alla
    scheda. (5) Il commit che corregge un difetto chiude la sua scheda. (6, 7)
    I test confrontano le copie con le fonti e i nomi citati con quelli
    definiti.
  ],
)

== I manpage <regole-man>

Una pagina per argomento, come oggi. Le pagine sono il riferimento completo
per chi usa l'editor: ogni nome pubblico --- funzione, comando, variabile,
modo --- ha una pagina-casa che lo elenca nella SYNOPSIS e ne spiega l'uso.
Una docstring non contiene niente che manchi alla pagina.

- *SYNOPSIS* elenca i nomi di cui la pagina è la casa: `(nome ARGOMENTI)` per
  funzioni e comandi, il nome da solo per le variabili. Ogni nome pubblico
  compare nella SYNOPSIS di una sola pagina. Da qui `C-h f` e `C-h v`
  ricavano il rimando «Manual:», che quindi non si scrive a mano.
- *EXAMPLES* c'è in ogni pagina: esempi completi, che si possono incollare in
  `init.lisp` o in `M-:`, ciascuno preceduto da una frase che dice a che cosa
  serve.
- *KEYS* e *KEYS IN … MODE* restano come oggi: le controlla `guide_tests`.
- Le sezioni intermedie seguono ciò che il lettore vuole fare, non la
  struttura del codice. Il funzionamento interno va nel manuale per
  sviluppatori; la pagina dice ciò che l'utente vede e può fare, limiti
  compresi.
- In inglese, righe entro 78 colonne, intestazioni in maiuscolo a colonna 0 e
  corpo rientrato di quattro spazi: è ciò che `manpage-mode` riconosce e
  colora.

Il modello:

```
NAME
    dired - a directory in a buffer

SYNOPSIS
    (dired DIRECTORY)
    (dired-find-recursive)
    dired-sort
    find-file-recursive-limit

DESCRIPTION
    What it is for, and the idea behind it, in a few paragraphs.

OPENING A DIRECTORY
    One heading for each thing the reader wants to do.

EXAMPLES
    A sentence saying what the example is for, then the example,
    complete and ready to paste into init.lisp or M-:.

        (setq dired-sort 'type)        ; directories first

KEYS
    C-x d                   dired

KEYS IN DIRED MODE
    <ret>                   dired-find-file

SEE ALSO
    files, buffers
```

== Le docstring <regole-docstring>

Valgono per le primitive (le costanti `*_DOC` di `primitives/`) e per le
funzioni, i comandi e le variabili dei moduli Lisp. Seguono le convenzioni di
Emacs, come già oggi.

```
(name ARGS): One sentence, in the imperative, saying what it does.

At most one short paragraph: what the arguments are, what comes back,
what is refused.

Example:
  (name ...)
```

- La prima riga basta da sola: è ciò che mostrano gli elenchi e i
  completamenti. Per le primitive comincia con la chiamata, `(name ARGS):`,
  come `doc_faithfulness_tests` già controlla.
- Gli argomenti in maiuscolo; gli altri nomi citati come #raw("`name'"), la
  convenzione di Emacs: è così che il test dei nomi citati li riconosce.
- Un esempio quando l'uso non è ovvio.
- Di norma non più di 600 caratteri; un test rifiuta quelle oltre 1 000.
- Niente architettura, storia o motivazioni lunghe: vanno nella pagina o nel
  manuale. Alla pagina porta il rimando che `C-h f` aggiunge da sé.
- Le funzioni private, con `--` nel nome, hanno una riga che dice che cosa
  restituiscono. Un ragionamento interno è un commento `;;`, non una
  docstring.
- Le impostazioni che il Rust legge --- variabili che nessun modulo
  dichiara con `defvar` --- hanno la docstring nel registro `SETTINGS` di
  `editor/settings.rs`, che all'avvio la mette dove la mette `defvar`;
  il loro valore iniziale si imposta in `create_global_env`, così `C-h v`
  mostra anche quello. La docstring non ripete il valore iniziale.
- Un'impostazione di un modulo si dichiara con `defvar` e la sua docstring,
  non con un `setq` commentato: così `C-h v` la spiega e i test la vedono.

== Il rustdoc <regole-rustdoc>

Segue le Rust API Guidelines e le convenzioni della RFC 1574.

- *La riga di riassunto* è una frase alla terza persona («Returns …»,
  «Creates …») che basta da sola: è ciò che `cargo doc` mostra negli elenchi.
- Poi, se serve, un paragrafo breve. Per gli elementi dell'interfaccia
  pubblica (i `pub use` di `lib.rs`): le sezioni `# Errors`, `# Panics` e
  `# Safety` quando si applicano, e `# Examples`.
- I tipi e le funzioni citati sono link (#raw("[`Log::flush`]")) e devono
  risolversi: `cargo doc --document-private-items` senza avvisi. Oggi ce ne
  sono 18.
- Campi e varianti: una riga quando il nome non basta, niente quando basta.
- *La documentazione di modulo* (`//!`) dice in una-cinque righe di che cosa
  il modulo è responsabile, e rimanda al capitolo del manuale con la sua
  etichetta: `Design: doc/architettura-editor.typ, <sfondo>.` Un test
  controlla che l'etichetta esista.
- In inglese, come oggi.

== I commenti nel codice <regole-commenti>

#ref-table(
  columns: (1fr, 1fr),
  header: ([Restano], [Escono]),
  [
    - un'invariante su cui il codice conta e che i tipi non esprimono;
    - l'ordine in cui si prendono i lock, il thread su cui una funzione gira;
    - `SAFETY:` prima di ogni `unsafe`;
    - un perché non ovvio, in una o due righe, con il rimando alla decisione
      se ne serve di più;
    - il motivo di un aggiramento: un terminale, un sistema operativo;
    - `TODO(ID): …`, dove ID è una scheda del rapporto o un'etichetta come
      quelle già in uso (`uncertain`, `improve`).
  ],
  [
    - ciò che il codice dice già;
    - la storia: «used to», «was … until», «the bug this fixes»;
    - le motivazioni lunghe, le sezioni `# Why`: diventano decisioni del
      manuale;
    - le spiegazioni per l'utente: vanno nel manpage;
    - i riferimenti a cose che non ci sono più;
    - gli ID di problemi già risolti.
  ],
)

Nei moduli Lisp l'intestazione `;;;` dice in una-cinque righe che cosa il
modulo aggiunge, e rimanda alla sua pagina e al capitolo del manuale.

== Il manuale per sviluppatori <regole-manuale>

- Un solo manuale, `doc/architettura-editor.typ`, con un capitolo per
  sottosistema. Ogni capitolo dice di che cosa il sottosistema è responsabile
  e ne descrive le strutture dati principali, i thread e i lock, i flussi
  (con diagrammi), come si estende (le ricette) e le decisioni.
- Una decisione è una voce breve: il problema, la scelta, che cosa comporta,
  l'alternativa scartata e perché. Prende il posto delle sezioni `# Why` del
  codice.
- I diagrammi usano gli aiuti di `style.typ`. Sono obbligatori almeno per gli
  strati e le dipendenze dei moduli, i thread, l'ordine dei lock, il percorso
  da un tasto a un comando, il lavoro asincrono e la fotografia del frame.
- Il codice si cita per nome di funzione o di tipo, non per numero di riga,
  che invecchia a ogni modifica. Gli estratti sono brevi.
- Le etichette (`<sfondo>`) non cambiano, perché il codice vi rimanda.
- In italiano, come oggi. Il PDF si rigenera nello stesso commit della
  sorgente.

== Il rapporto sui problemi <regole-rapporto>

- Una scheda per difetto, con un ID stabile, come oggi.
- Un campo nuovo, `stato`: `aperto` oppure `risolto`, con il commit che lo
  risolve. Le schede risolte escono dal riepilogo e passano in un'appendice
  compatta.
- Il codice cita un ID solo in un `TODO(ID)`, e solo finché la scheda è
  aperta.

== La storia e i commit <regole-storia>

Un commento che racconta la storia si cancella. Prima si guarda se contiene
una decisione ancora valida: se sì, la decisione va nel manuale. Il resto
della storia sta nel messaggio di commit, che dice che cosa cambia, perché,
che cosa c'era prima e quale scheda chiude.

#caution[
  Per questo le patch si applicano con `git am`, che conserva il loro
  messaggio di commit (decisione 7 del @domande).
]

== I test <regole-test>

- Il nome di un test descrive il comportamento, come già si fa:
  `a_variable_set_but_empty_counts_as_not_set`.
- Un commento in un test spiega in una o due righe un caso non ovvio. La
  storia del difetto che il test previene va nel commit.
- I test di documentazione sono quelli che controllano le copie
  (@copie-controllate).

== Le copie controllate <copie-controllate>

#ref-table(
  columns: (4.6cm, 4.4cm, 1fr),
  header: ([Copia], [Fonte], [Test]),
  [le sezioni KEYS], [i keymap], [`guide_tests` (esiste)],
  [i SEE ALSO], [la directory `man/` e le funzioni], [`guide_tests` (esiste)],
  [l'indice di `guide.txt`], [la directory `man/`], [`guide_tests` (esiste)],
  [la prima riga delle docstring delle primitive], [il nome della primitiva], [`doc_faithfulness_tests` (esiste)],
  [la sezione MODULES di `rsedit.txt`], [`DEFAULT_INIT_LISP`], [`guide_tests`],
  [le SYNOPSIS], [i nomi definiti], [`guide_tests`],
  [i nomi citati in pagine e docstring], [i nomi definiti], [`doc_faithfulness_tests`],
  [i rimandi dal codice al manuale], [le etichette typst], [`doc_pointer_tests`],
)

I test partono con l'elenco delle eccezioni di oggi --- un nome senza pagina,
una docstring troppo lunga --- che può solo accorciarsi. Gli elenchi stanno in
`core/src/tests/doc_exceptions/`, raggruppati per il passo che li smaltisce, e
un nome che vi resta dopo essere stato sistemato fa fallire il test. Così
nessun passo deve correggere tutto subito, e nessuno può peggiorare le cose.

== La lista di controllo di ogni patch <lista>

#key[
  - Comportamento visibile cambiato → la pagina, con i suoi esempi.
  - Nome pubblico nuovo → la docstring e la SYNOPSIS della sua pagina.
  - Struttura, thread o lock cambiati → il capitolo del manuale e il suo
    diagramma.
  - Una scelta non ovvia → una voce tra le decisioni.
  - Difetto corretto → la scheda passa a `risolto` e il `TODO(ID)` si toglie.
  - Nessun commento storico, nessun rimando a cose tolte.
  - Test verdi, compresi quelli di documentazione; `cargo doc` senza avvisi;
    i PDF rigenerati.
]

== Che cosa si fa dei commenti di oggi <classificazione>

Ogni commento e ogni docstring esistente rientra in uno di questi casi. La
tabella è lo strumento dei passi delle fasi B e C.

#ref-table(
  columns: (3.7cm, 4.2cm, 1fr),
  header: ([Il commento…], [Che cosa se ne fa], [Esempio, revisione `58d0b0d`]),
  [ripete il codice], [si cancella], [---],
  [racconta la storia], [si cancella; una decisione ancora valida va nel manuale], [`managers/log.rs`: «`log_diagnostic` used to take the list's lock and, still holding it, `write_all` to the file…»],
  [cita un problema risolto], [si toglie l'ID, la regola resta], [`Log::flush`: «A write that fails is dropped, never a panic (AVV-3)»],
  [spiega all'utente come usare qualcosa], [va nel manpage], [`DEFAULT_INIT_LISP` rimanda a `common-keymaps.lisp` per spiegare `auto-save` e `watch-files`; la docstring di `background-call` (@esempi)],
  [motiva una scelta di progetto], [va tra le decisioni del manuale; sul posto, se serve, una riga e il rimando], [`paths.rs`: «Why macOS gets `~/.config` and `~/.cache`»],
  [enuncia un'invariante, un ordine di lock, un thread], [resta, in una-tre righe], [`Log::flush`: «The file's lock is taken before the lines are…»],
  [descrive lavoro da fare], [una scheda del rapporto e `TODO(ID)`], [`paths.rs`: il TODO sulla scrivibilità della directory],
  [è falso, o cita cose che non esistono], [si corregge o si cancella], [`lib.rs` elenca un modulo `lisp/`, che è nel crate `risp`; cinque rimandi a `rsedit-man` (@nomi)],
)

== Tre esempi <esempi>

=== Una docstring: `background-call`

Oggi la docstring è lunga 3 689 caratteri. Eccone l'inizio:

```
(background-call NAME JOB &optional ON-PROGRESS ON-DONE INTERVAL): Do JOB in
the background under the name NAME, and return NAME at once without waiting
for it.

This is how anything that may take longer than a frame is started. JOB is
either a function of no arguments -- called once, in the background -- or a
`fiber', which is resumed a step at a time and is the shape to use when the
work can be done in pieces:
  [un esempio di nove righe e altri otto paragrafi]
```

Quasi tutto ripete la pagina `background`, che ha lo stesso esempio e le
stesse regole. Dopo il riordino:

```
(background-call NAME JOB &optional ON-PROGRESS ON-DONE INTERVAL): Run JOB in
the background under NAME, and return NAME at once.

JOB is a function of no arguments, called once, or a `fiber', resumed a step
at a time. ON-PROGRESS is called with NAME after each step but the last, and
ON-DONE with NAME and t or nil when JOB ends; both run between commands. A
job started under a name in use replaces the one running. INTERVAL is how
many milliseconds a plain (yield) waits: 40 by default.

Example:
  (defun my-done (name ok) (message "%s finished: %s" name ok))
  (background-call 'my-sum (lambda () (setq my-sum (my-slow-sum))) nil 'my-done)

Manual: background
```

L'ultima riga non è scritta nella docstring: la aggiunge `C-h f`, dalla
SYNOPSIS della pagina. La formulazione definitiva si scrive nel passo 8.

=== Un rustdoc: `app_dir`

Oggi:

````rust
/// The directory OPTIONS asks for, where this system keeps it -- made first,
/// when it is missing and OPTIONS says to. See the module documentation for
/// where that is.
````

Dopo, con le sezioni della RFC 1574:

````rust
/// Returns the directory `options.dir` names on this system, creating it and
/// its parents first when it is missing and `options.create` is set.
///
/// Where each directory is: the `configuration` manual page.
///
/// # Errors
///
/// [`DirError::Undetermined`] when nothing in the environment says where the
/// directory is; [`DirError::NotFound`] when it is missing and `create` is
/// false; [`DirError::NotADirectory`] or [`DirError::CannotCreate`] otherwise.
///
/// # Examples
///
/// ```no_run
/// use rsedit_core::{AppDir, DirOptions, app_dir};
///
/// let cache = app_dir(DirOptions { dir: AppDir::Cache, create: true })?;
/// # Ok::<(), rsedit_core::DirError>(())
/// ```
````

La tabella delle precedenze, oggi nella documentazione del modulo, è
un'informazione per l'utente: passa nella pagina `configuration`, che ne ha
già una parte. Le due sezioni `# Why`, sul macOS e sul Windows, diventano
decisioni del capitolo sui file dell'editor.

=== Una documentazione di modulo: `managers/log.rs`

Oggi sono 29 righe:

```
//! Diagnostics: what has been logged, and where a copy of it goes.
//!
//! # Why these two are one thing
//!   [7 righe: la lista e il file si aggiornano insieme, e perché]
//! # Who writes the file
//!   [10 righe: append non scrive, flush sì, e lo chiama il watchdog]
//!
//! `log_diagnostic` used to take the list's lock and, still holding it,
//! `write_all` to the file, which put a disk write inside a lock every
//! diagnostic in the editor had to queue behind. The flush writes with the
//! list's lock given back.
```

Dopo:

```
//! Diagnostics: the list of what has been logged, and the file it is
//! copied to. [`Log::append`] only records; [`Log::flush`], which the
//! WorkManager's watchdog calls, writes what is owed.
//!
//! Design: doc/architettura-editor.typ, <log>.
```

Il primo `# Why` diventa la decisione «La lista e il file si aggiornano
insieme». Chi scrive il file, e su quale thread, va nel capitolo sui thread.
La storia si cancella (@regole-storia).

// ===========================================================================
= L'inventario <inventario>
// ===========================================================================

Misure sulla revisione `58d0b0d`, ottenute con script che contano le righe e
cercano i nomi; sono fotografie, non giudizi.

== Quanto c'è

#ref-table(
  columns: (5.0cm, 1.9cm, 3.1cm, 2.1cm, 1fr),
  header: ([Parte], [codice], [documentazione], [commenti], [quota]),
  [Rust, `core/src` e `src`, senza i test], [23 832], [8 433], [2 699], [32 %],
  [Rust, test], [34 439], [2 699], [3 020], [14 %],
  [Lisp, `core/lisp` (22 moduli)], [3 033], [1 004], [1 632], [46 %],
  [manpage, `man/` (22 pagine)], [---], [2 225], [---], [---],
  [typst, `doc/` (5 sorgenti)], [---], [5 865], [---], [---],
)

Righe. Nel Rust «documentazione» sono le righe `///` e `//!`, «commenti» le
righe `//`; nel Lisp «documentazione» sono le docstring e «commenti» le righe
`;`. La quota è documentazione più commenti sul totale. Le docstring delle
primitive sono stringhe Rust e qui contano come codice: 2 491 righe.

== I commenti del Rust

#ref-table(
  columns: (3.6cm, 2cm, 3.1cm, 2.1cm, 1fr),
  header: ([Modulo], [codice], [documentazione], [commenti], [quota]),
  [`primitives/`], [11 628], [1 604], [1 030], [18 %],
  [`editor/`], [2 423], [1 527], [519], [46 %],
  [`work/`], [1 996], [667], [72], [27 %],
  [`buffer/`], [1 533], [885], [175], [41 %],
  [`modes/`], [1 403], [682], [186], [38 %],
  [`managers/`], [1 366], [1 133], [212], [50 %],
  [`ui/`], [1 348], [724], [187], [40 %],
  [`text/`], [755], [491], [64], [42 %],
  [`src/` (la TUI)], [629], [134], [170], [33 %],
  [`input.rs`], [241], [120], [8], [35 %],
  [`feature/`], [211], [201], [56], [55 %],
  [`commands.rs`], [136], [104], [3], [44 %],
  [`paths.rs`], [132], [102], [7], [45 %],
  [`lib.rs`], [31], [59], [10], [69 %],
)

La quota bassa di `primitives/` è un effetto del conteggio: le sue docstring
contano come codice.

Le righe di commento fuori dai test sono 11 132. Di queste, 3 269 stanno in
128 blocchi di almeno 15 righe consecutive, e 17 blocchi superano le 40.
Alcune formule segnalano i commenti da rivedere:

#ref-table(
  columns: (6.4cm, 2.6cm, 1fr),
  header: ([Formula], [fuori dai test], [nei test]),
  [storia: «used to», «no longer», «the bug this…»], [78], [57],
  [sezioni `# Why`], [145], [16],
  [«for now», «today», «not yet»], [38], [5],
  [ID del rapporto (`AVV-3`, …)], [2], [5],
  [`TODO`], [4], [0],
)

Le righe storiche sono per lo più storia vera («It used to keep the renderer
waking every 40 ms»), con qualche falso positivo («text that no longer
exists»). I quattro `TODO` sono in `editor/log.rs`, `paths.rs`,
`primitives/ui.rs` e `src/main.rs`; nessuno rimanda a una scheda.

== Docstring e rustdoc

#ref-table(
  columns: (5.6cm, 1fr),
  header: ([Misura], [Valore]),
  [docstring delle primitive], [296 costanti, 2 491 righe di sorgente],
  [lunghezza: mediana, 90° percentile, massimo], [469, 1 089, 3 689 caratteri (`background-call`)],
  [oltre 1 500 caratteri], [8: `background-call`, `set-transient-keymap`, `define-worker`, `shell-command-start`, `directory-files-recursive` e altre tre],
  [con un esempio; con un rimando a una pagina], [230; 2],
  [docstring dei moduli Lisp], [251; mediana 161 caratteri, massimo 1 656 --- una funzione privata, `electric-pair--closed-ahead-p`],
  [definizioni Lisp private (`--`)], [212 su 319],
  [blocchi `///`], [1 564: 919 di una-tre righe, 530 di quattro-dieci, 98 di 11--20, 17 oltre le 20],
  [documentazione di modulo `//!`], [103 file: 16 senza, 16 di una-cinque righe, 20 di 6--20, 50 di 21--60, 1 oltre (`primitives/lisp_job.rs`, 78)],
  [avvisi di `cargo doc --document-private-items`], [18, quasi tutti link a moduli privati],
)

== I moduli Lisp

#ref-table(
  columns: (4.4cm, 1.5cm, 1.7cm, 2.2cm, 2.2cm, 1fr),
  header: ([Modulo], [righe], [codice], [commenti], [docstring], [intestazione]),
  [`vim`], [1 067], [1 006], [19], [24], [5],
  [`dired`], [618], [221], [174], [142], [51],
  [`help`], [565], [331], [72], [88], [34],
  [`completion`], [512], [162], [158], [123], [45],
  [`manpage`], [485], [162], [119], [134], [32],
  [`indent`], [410], [161], [106], [85], [32],
  [`common-keymaps`], [399], [108], [250], [2], [0],
  [`electric-pair`], [375], [122], [87], [110], [16],
  [`compile`], [334], [155], [95], [43], [26],
  [`theme`], [206], [96], [46], [34], [30],
  [`buffer-list`], [198], [104], [47], [18], [29],
  [`completion-at-point`], [185], [44], [61], [55], [35],
  [`occur`], [180], [89], [46], [15], [26],
  [`risp-mode`], [143], [34], [68], [19], [32],
  [`preview`], [139], [50], [55], [19], [34],
  [`rust-mode`], [131], [35], [71], [3], [12],
  [`debug`], [118], [40], [43], [22], [0],
  [`shell`], [105], [45], [34], [9], [28],
  [`cc-mode`], [95], [44], [24], [3], [1],
  [`commands`], [80], [9], [20], [39], [0],
  [`clipboard`], [47], [6], [31], [6], [22],
  [`minibuffer`], [31], [9], [6], [11], [0],
)

Righe; «intestazione» è il blocco `;;;` in testa al file, compreso nei
commenti. Le intestazioni sono saggi di progetto: quella di `dired` spiega in
51 righe perché lo stato è il buffer stesso e perché il formato è così
semplice. Le docstring delle funzioni private fanno spesso da commento:
quella di `dired--parent` (992 caratteri) racconta come la funzione era
scritta prima.

== I manpage

Le 22 pagine sono lunghe 2 225 righe. Hanno tutte NAME e DESCRIPTION; 21 hanno
SEE ALSO, 7 una sezione KEYS, 3 una SYNOPSIS, nessuna una sezione EXAMPLES.
Gli esempi di codice sono concentrati in tre pagine (`writing-commands`,
`configuration`, `background`); dieci pagine non ne hanno.

#ref-table(
  columns: (5.6cm, 1fr),
  header: ([Misura], [Valore]),
  [nomi pubblici: primitive e definizioni dei moduli], [487, di cui 234 citati in almeno una pagina (48 %)],
  [comandi], [180, di cui 129 citati (72 %)],
  [completamente coperti], [`overlays`, `virtual_text`, `workers`],
  [quasi scoperti], [`completion.lisp` 1 su 11, `primitives/completion.rs` 1 su 8, `help.rs` 1 su 6, `mouse.rs` 1 su 6, `minibuffer.rs` 2 su 12, `results.rs` 2 su 9; nessuno di `completion-at-point`, `preview`, `clipboard`],
  [impostazioni lette dal Rust], [18: 4 citate in una pagina, 1 sola (`help-key`) con una docstring per `C-h v`],
  [moduli senza una pagina che li descriva], [`dired` (solo dentro `files`), `shell`, `indent`, `electric-pair`, `clipboard`, `preview`, `cc-mode`, `completion-at-point`],
  [argomenti senza pagina], [il log e la diagnostica, il mouse, l'indentazione],
)

La copertura conta un nome come coperto se una pagina lo nomina anche solo di
passaggio: le pagine che lo spiegano davvero sono meno.

== I documenti in `doc/` e nella radice

#ref-table(
  columns: (4.6cm, 1.3cm, 1fr, 3.3cm),
  header: ([File], [righe], [Stato], [Destinazione]),
  [`architettura-editor.typ`], [2 642], [Buono fino al lavoro asincrono. Il capitolo «Il lavoro in background» descrive lo scheduler tolto (`BackgroundScheduler`, `RunNow`, `WorkerMessage`, `background/worker.rs`); il log è ancora «nella directory corrente»; ritmi di 40 ms che non esistono più; l'appendice delle primitive è una copia generata e già vecchia.], [il manuale per sviluppatori: capitoli riscritti nella fase C],
  [`workmanager.typ`], [786], [Documento di progetto: si dichiara «da implementare», mentre le patch 1--9 l'hanno implementato; il piano e le decisioni sono storia.], [nel capitolo sul lavoro asincrono, poi si toglie],
  [`lisp-fibers.typ`], [870], [In inglese. Descrive l'interno del crate `risp`: tutti i suoi percorsi e 16 dei suoi identificatori sono là.], [il repository `risp` (decisione 5)],
  [`rapporto-problemi.typ`], [1 244], [80 schede sulla revisione `08a853d`, nessuna segnata come risolta. Le 29 delle aree LET e INT riguardano l'interprete, ora in `risp`; rimanda ad `architettura-interprete.typ`, spostato in `risp` con `10598fa`. Almeno AVV-1, AVV-3, AVV-4 e BKG-1 sembrano risolte dalle patch.], [aggiornato nel passo 10],
  [`style.typ`], [323], [Gli aiuti condivisi per i diagrammi.], [resta],
  [`lisp-interpreter.pdf` \ `lisp-interpreter-flow.pdf`], [---], [PDF senza sorgente, dal 24 settembre.], [da togliere (decisione 4)],
  [`README.org`], [---], [Dice `Ctrl+S` per salvare e `Ctrl+Q` per uscire (sono `C-x C-s` e `C-x C-c`; `C-s` cerca); ripete metà della pagina `vim`; rimanda a `M-x rsedit-man`, che non esiste.], [presentazione e compilazione, poi i rimandi],
  [`PERFORMANCE.md`], [---], [Valido, salvo l'ultimo paragrafo, un'istruzione di una volta sola per applicare due patch.], [resta, senza l'ultimo paragrafo],
  [`man/README.md`], [---], [Dice che le pagine sono per argomenti e che la docstring «is the reference»: il contrario della regola nuova.], [riscritto nel passo 1],
)

== Lo stesso fatto in più posti <copie>

#ref-table(
  columns: (3.4cm, 5.2cm, 1fr),
  header: ([Fatto], [Dove], [Come divergono]),
  [I moduli caricati all'avvio], [`DEFAULT_INIT_LISP` (22 moduli), `build.rs` (22), `shipped_lisp_tests` (19), `doc_faithfulness_tests` (18), la sezione MODULES di `rsedit.txt` (19), `architettura-editor.typ` (21)], [A `shipped_lisp_tests` mancano `cc-mode`, `preview` e `completion-at-point`; a `doc_faithfulness_tests` mancano `preview`, `theme`, `vim` e `completion-at-point`, le cui docstring quindi non sono controllate. La pagina non ha `cc-mode`, `preview` e `vim`; il manuale non ha `vim`.],
  [L'elenco delle pagine], [la directory `man/` (22), l'elenco di `guide_tests` (21), l'indice di `guide.txt`], [Manca `vim`: la guida non la elenca, `rsedit.txt` non la cita e i suoi tasti non sono controllati.],
  [Dove sta la configurazione], [`paths.rs`, `configuration.txt`, `architettura-editor.typ`], [Pagina e manuale non dicono nulla della cache e del log; il manuale ha il log nella directory corrente.],
  [Le impostazioni], [le costanti dei moduli Rust, la tabella del manuale, i commenti di `DEFAULT_INIT_LISP` e di `common-keymaps.lisp`], [`configuration.txt` dice che `C-h v` spiega ogni variabile: vale per 1 impostazione su 18.],
  [L'ordine dei moduli], [`DEFAULT_INIT_LISP`, `configuration.txt`], [«The order matters» contro «None of them needs any other».],
  [Le primitive], [il registro (`into.function`, `into.command`), l'appendice del manuale], [L'appendice ne dà 4 a `workers` (sono 9) e 47 a `edits` (sono 48).],
  [`background-call` e i lavori Lisp], [la docstring, `background.txt`, `workmanager.typ`, il manuale, le 78 righe di `//!` di `primitives/lisp_job.rs`], [Stesso esempio e stesse regole in tre posti; il manuale descrive lo scheduler di prima.],
  [La modalità vim], [`vim.txt`, `README.org`], [Il README ripete comandi e impostazioni.],
)

== Nomi citati che non esistono <nomi>

- `rsedit-man`, in cinque posti: `man/README.md`, `README.org`, la docstring
  di `data-directory`, un commento di `build.rs` e `vim.lisp`. Il comando è
  `manpage`.
- Nelle docstring: `read-command-args` (in `primitives/commands.rs`),
  `search-forward` (in quella di `scan-buffer`), `find-file-recursive` (il
  comando è `dired-find-recursive`), `default-directory` (in quella di
  `grep`). Chi le legge con `C-h f` e segue il nome non trova nulla.
- Nei commenti: `set-mode-keywords` (`rust-mode.lisp`); nomi di Emacs al
  posto di quelli di rsedit, come `self-insert-command`.
- Nei documenti typst: 8 identificatori del manuale e 5 di `workmanager.typ`
  non esistono più, come il `background/mod.rs` citato dal rapporto; 16
  percorsi del rapporto e tutti quelli di `lisp-fibers.typ` sono ora nel
  crate `risp`.

== Difetti trovati durante l'inventario <difetti>

Non sono di documentazione, ma l'inventario li ha fatti emergere.

+ *`:help` di vim non fa nulla.* `vim.lisp` chiama `rsedit-man` solo se
  esiste, e non esiste: il comando resta muto.
+ *La pagina `vim` non si raggiunge dalla guida* e i suoi tasti non sono
  controllati (@copie).
+ *I moduli `theme`, `vim`, `preview` e `completion-at-point` sfuggono a
  `doc_faithfulness_tests`*, che carica un elenco suo di moduli.
+ *`configuration.txt` consiglia `(eval-file "clipboard")`*, che
  l'`init.lisp` predefinito carica già.

// ===========================================================================
= Il piano <piano>
// ===========================================================================

#figure(
  canvas(height: 3.4cm)[
    #node(2.0cm, 1.1cm, 3.4cm, 1.4cm, [*A · regole e controlli* \ passo 1, fatto], fill: paper)
    #node(6.1cm, 1.1cm, 3.4cm, 1.4cm, [*B · manpage e docstring* \ passi 2--9, per area], fill: user-fill)
    #node(10.2cm, 1.1cm, 3.4cm, 1.4cm, [*C · codice e commenti* \ passi 10--21, per sottosistema])
    #node(14.2cm, 1.1cm, 3.4cm, 1.4cm, [*D · manuale* \ passo 22], fill: paper)
    #carrow((3.7cm, 1.1cm), (4.4cm, 1.1cm))
    #carrow((7.8cm, 1.1cm), (8.5cm, 1.1cm))
    #carrow((11.9cm, 1.1cm), (12.5cm, 1.1cm))
    #note(0.3cm, 2.15cm)[le regole entrano nel \ repository, i test \ fissano lo stato di oggi]
    #note(4.4cm, 2.15cm)[le pagine diventano \ complete; le docstring \ si accorciano]
    #note(8.5cm, 2.15cm)[il rapporto aggiornato, poi \ codice, commenti e capitolo \ del manuale, insieme]
    #note(12.5cm, 2.15cm)[i capitoli si uniscono; \ i documenti vecchi \ si ritirano]
  ],
  caption: [Le quattro fasi. Ogni passo è una patch, consegnata e verificata come le patch 1--9.],
)

L'ordine è quello concordato: prima i manpage insieme alle docstring, poi il
codice e i commenti modulo per modulo, infine il manuale. Con una
precisazione: i perché tolti dal codice nella fase C vanno subito nel
capitolo del manuale del loro sottosistema, così nessuna informazione resta
senza casa tra un passo e l'altro. La fase D unisce i capitoli, disegna i
diagrammi d'insieme e ritira i documenti vecchi (decisione 9).

== Fase A --- le regole e i controlli (passo 1, fatto)

+ `README.md` prende il posto di `README.org`: una presentazione essenziale
  del progetto, con i documenti e i loro destinatari. `man/README.md`
  descrive il modello di pagina del @regole-man.
+ *Un solo elenco dei moduli.* `build.rs` copia tutta `core/lisp/`; i test
  leggono l'elenco da `DEFAULT_INIT_LISP`; `guide_tests` confronta con quello
  la sezione MODULES di `rsedit.txt`, che ora ha tutti i 22 moduli.
+ *Un solo elenco delle pagine.* I test leggono la directory `man/`.
+ *I test nuovi*, con le eccezioni di oggi in `core/src/tests/doc_exceptions/`:
  459 nomi senza casa, 30 docstring oltre i 1 000 caratteri, 68 funzioni
  private di `vim.lisp` senza docstring, 11 nomi citati che l'editor non
  definisce (5 variabili lette dal Rust e mai dichiarate, 6 nomi sbagliati).
+ *Il rimando automatico.* `C-h f` e `C-h v` mostrano «Manual: pagina», e `K`
  o `RET` su quella riga apre la pagina.
+ *Le correzioni del @difetti.* `:help` di vim apre la pagina `vim`, che ora
  è nella guida, in `rsedit.txt` e nei test; tutti i moduli sono controllati.
  Le SYNOPSIS di `compile`, `themes` e `vim` elencano nomi, e i loro tasti
  sono passati in sezioni KEYS controllate.
+ Tolti i due PDF orfani e l'ultimo paragrafo di `PERFORMANCE.md`.

== Fase B --- i manpage e le docstring (passi 2--9)

Ogni passo copre un'area. Le pagine dell'area si riscrivono sul modello, con
SYNOPSIS, EXAMPLES e KEYS; le pagine mancanti si scrivono. Le docstring dei
nomi dell'area si accorciano sul modello. Le spiegazioni per l'utente che
stanno nei commenti del codice dell'area passano nelle pagine e dai commenti
escono. Le impostazioni dell'area ricevono una docstring, così che `C-h v`
le spieghi. L'elenco dei nomi senza casa si accorcia di conseguenza.

*Passo 2, fatto.* `configuration` è riscritta sul modello e corretta; la
pagina nuova `diagnostics` spiega l'area dei messaggi, il log, il file di log
nella cache, i comandi che falliscono e le righe del watchdog. Il registro
`SETTINGS` documenta le prime cinque impostazioni (`echo-message-timeout`,
`mode-line-format`, `display-line-numbers`, `display-line-numbers-width`,
`lisp-path`), e i passi seguenti vi aggiungono quelle della loro area. I nomi
di `commands.lisp` passano al passo 8, perché la loro pagina è
`writing-commands`.

*Passo 3, fatto.* `editing` è riscritta sul modello: muoversi, scrivere, le
coppie di delimitatori, le righe, i commenti, annullare. Le pagine nuove sono
due invece di una: `indentation`, come previsto, e `killing`, che prende da
`editing` il kill ring, la regione e gli appunti di sistema; in una pagina
sola la SYNOPSIS avrebbe superato i novanta nomi. `rectangles` e `macros`
seguono il modello, con i tasti nelle sezioni KEYS. Le 107 docstring delle sei
primitive dell'area e quelle dei tre moduli seguono il modello, e i
ragionamenti interni delle funzioni private sono diventati commenti `;;`.
Cinque impostazioni che i moduli assegnavano con `setq` (`tab-width`,
`tab-always-indent`, `electric-pair-mode`, `electric-pair-skip-self`,
`clipboard-sync`) sono ora `defvar` con docstring; `clipboard-sync-toggle` è
un comando. I commenti di `common-keymaps.lisp` che spiegavano all'utente i
tasti dell'area sono diventati rimandi alle pagine.

Le pagine e le docstring dell'area dicevano cose false, ora corrette:

- `current-column` contava «le colonne, non i caratteri»: conta i caratteri,
  come `current-indentation`, e un tab vale uno; l'avvertenza in testa a
  `indent.lisp` era sbagliata allo stesso modo;
- `C-u 5` seguito da un carattere lo inserisce una volta sola, non cinque;
- `yank` non mette il mark all'inizio del testo inserito;
- `transpose-lines` sposta in su la riga corrente, e point con essa;
- `insert-pasted-text` restituisce sempre nil, non t;
- `set-mark` prende un secondo argomento, INCLUSIVE, che la firma taceva;
- `rectangles` diceva che un tab occupa una cella dello schermo (TUI-3 nel
  rapporto);
- tre C-k di fila prendono una riga e mezza, non tre righe (`first-steps`);
- l'esempio di `define-kbd-macro` legava `C-c t`, il tasto dei temi;
- i commenti dicevano il kill ring «da fare» e `risp-mode` «assente».

I tasti Canc, Inizio, Fine, PagSu e PagGiù e i tasti funzione non arrivano
all'editor (CMD-2 nel rapporto): `editing` lo dice tra i limiti, e il legame
di `<delete>` in `electric-pair.lisp` resta senza effetto finché la scheda è
aperta.

*Passo 4, fatto.* `files`, `buffers` e `windows` sono riscritte sul modello, e
`dired` e `mouse` sono nuove, come previsto. Il registro `SETTINGS` documenta
nove impostazioni dell'area, con il loro valore iniziale: `auto-save`,
`auto-save-interval`, `auto-save-directory`, `watch-files`,
`watch-file-interval`, `*open-directory-callback*`, `mouse-mode`,
`window-separator` e `after-resize-hook`. Le spiegazioni che stavano nei
commenti di `common-keymaps.lisp` e dell'`init.lisp` predefinito sono passate
in `files`. Le impostazioni di `dired.lisp` erano `defconst` e ora sono
`defvar`, così un valore scelto prima che il modulo si carichi resta. Le 67
docstring delle primitive pubbliche dell'area e quelle dei due moduli seguono
il modello. `all-buffer-names` trova casa in `buffers` ora invece che nel
passo 6.

Le pagine e le docstring dell'area dicevano cose false, ora corrette:

- due finestre sullo stesso buffer non condividono il punto: ciascuna ricorda
  il suo;
- un file cambiato sul disco viene riletto quando il buffer non ha modifiche,
  e la barra non segnala nulla: `files` diceva il contrario su entrambi i
  punti;
- chiudere un buffer porta le sue finestre sul buffer usato più di recente,
  non su `*scratch*`;
- `delete-window` accetta un id, che la firma taceva;
- una `defun` non sostituisce una primitiva, quindi il doppio clic non si
  ridefinisce, come invece prometteva la docstring di
  `mouse-start-selection`;
- `C-g` non chiude un prompt (MIN-2 nel rapporto) né una domanda a un tasto,
  mentre `guide` e `first-steps` dicevano di sì;
- `first-steps` mandava a `M-x mouse-mode-toggle` per accendere il mouse, che
  l'`init.lisp` predefinito accende già;
- i commenti dicevano `watch-file-interval` e `auto-save-interval` letti una
  volta all'avvio: si leggono a ogni giro;
- i commenti dicevano che ogni mezzo minuto si copia ogni buffer modificato:
  ogni mezzo minuto se ne guardano quattro, a turno, e contano tutti, anche
  quelli senza file come `*scratch*`; con dodici buffer un file si copia ogni
  minuto e mezzo.

Il passo ha trovato due difetti del codice, che non corregge: li corregge la
patch descritta dopo il passo 5.

- `auto-save-directory` non espande `~` e non crea la cartella, quindi il
  valore che l'`init.lisp` predefinito suggeriva, `"~/.cache/rsedit"`, non
  funzionava. Il suggerimento ora è `(expand-file-name "~/.cache/rsedit")`, e
  la pagina `files` dice che la cartella deve esistere.
- `BUFFERS_PER_TURN`, in `autosave.rs`, dice quanti buffer un giro può
  scrivere, ma conta quelli guardati: chi tiene aperti molti buffer aspetta la
  copia ben più di mezzo minuto. La pagina ora lo dice.

*Passo 5, fatto.* `searching` e `compile` sono riscritte sul modello. La
pagina nuova è `regexps`, che il piano non prevedeva: la sintassi delle
espressioni regolari non era scritta da nessuna parte, e le funzioni dei dati
del match (`string-match` e le altre sei), `save-match-data` e `scan-buffer`
vi trovano casa. Il registro `SETTINGS` documenta `case-fold-search`,
`case-replace` e `*replace-read-function*`, e le ultime due hanno ora un
valore all'avvio. In `compile.lisp` `compilation-patterns` passa da `setq` a
`defvar` con docstring, e le altezze delle finestre, `compilation-window-height`
e `occur-window-height`, da `defconst` a `defvar`. Le 41 docstring delle
primitive dell'area e quelle dei due moduli seguono il modello. I commenti di
`common-keymaps.lisp` sulla ricerca sono diventati rimandi, e il suo
`(setq case-fold-search t)`, che il Rust fa già, non c'è più.

Le pagine e le docstring dell'area dicevano cose false, ora corrette:

- durante `M-%`, Return non si limita a fermare la sostituzione: la ferma e
  poi va a capo, come fa ogni tasto che non risponde alla domanda;
  `searching` lo dava come il tasto per fermarsi, e taceva `q`;
- `C-s` evidenzia solo la corrispondenza su cui si trova, non tutte;
- `replace-string` e `replace-regexp` non sostituiscono «dappertutto», ma dal
  cursore in avanti, o nella regione; senza corrispondenze restituiscono nil,
  non un numero;
- `string-match` distingue sempre maiuscole e minuscole, qualunque sia
  `case-fold-search`, e per lei `^` e `$` sono gli estremi della stringa;
- solo la ricerca nei file si riempie man mano, non quella nel buffer;
- `grep` cercava «sotto `default-directory`», che non esiste: cerca nella
  cartella in cui l'editor è partito; `grep--scan` rimandava a
  `find-file-recursive` e `scan-buffer` a `search-forward`, che non esistono;
- `scan-buffer` prometteva i gruppi `\1` e seguenti a chi legge le voci, che
  non li portano.

Il passo ha trovato quattro difetti del codice, che non corregge: li corregge
la patch descritta qui sotto.

- dopo `C-s` e Return il messaggio dice «Mark saved where search started», ma
  il mark resta all'altra estremità della corrispondenza;
- `occur--display` mostra la lista nella striscia in basso e anche nella
  finestra in cui si lavorava, al posto del buffer; per la stessa ragione `o`
  non lascia vedere la corrispondenza;
- Return nell'output di una compilazione divide la striscia: il file la
  occupa quasi tutta, l'output resta alto una riga e il cursore passa al
  file; `compilation-goto` non sposta il punto da cui riparte `M-g n`;
- la prima riga dell'output, il comando, è letta come un luogo quando il
  comando ne contiene uno.

*I difetti corretti.* Una patch a parte corregge i sei difetti dei passi 4 e
5, e altri quattro trovati mentre li verificava; le pagine non li descrivono
più tra i limiti, e ognuno ha un test che fallisce senza la correzione:

- dopo `C-s` e Return il mark va dove la ricerca è partita, come dice il
  messaggio;
- la lista di `M-s o` e `M-s g` si apre nella striscia in basso e ne prende
  il focus, mentre la finestra in cui si lavorava resta com'era; Return e `o`
  aprono la corrispondenza in quella finestra;
- Return nell'output di una compilazione apre il file nella finestra da cui
  la compilazione è partita, senza dividere la striscia, e `M-g n` riparte da
  lì;
- la riga del comando non è più letta come un luogo;
- `auto-save-directory` espande `~` e la cartella si crea alla prima copia;
- ogni giro dell'auto-save guarda tutti i buffer e scrive al più quattro
  copie;
- una seconda lista, o una seconda compilazione, apriva un'altra striscia
  sotto la prima: ora usa la stessa;
- nell'output di una compilazione `M-g n` segnava la riga sbagliata, quella
  che segue l'indice della voce, giusta solo per le liste di ricerca; ora una
  voce di `results-put` può dire, con un nono elemento, su quale riga della
  lista sta;
- `M-g n` su un file che non esiste apriva un buffer vuoto con quel nome:
  ora dice che il file manca, come già faceva Return nell'output;
- `C-h f` su una macro diceva che non è una funzione: ora la descrive, con
  argomenti e docstring. `defmacro` di `risp` lascia la docstring nel corpo,
  e la primitiva privata `help--macro-doc` la legge da lì.

`compilation-mark-priority` non c'è più: i segni sono quelli della
navigazione comune, e il codice di `compile.lisp` che li metteva da sé è
stato tolto con loro.

*Passo 6, fatto.* `completion`, `keymaps` e `getting-help` sono riscritte sul
modello. La pagina nuova è `minibuffer`, che il piano non prevedeva: il
prompt, la sua cronologia, `M-:`, l'anteprima di `C-x r t` e `minibuffer-read`
non stavano in nessuna pagina, e in `completion` la SYNOPSIS avrebbe superato
i cinquanta nomi. `regexp-opt` trova casa in `regexps`; `list-workers` passa
al passo 8, con la pagina `background`, che oggi non ha una SYNOPSIS. Il
registro `SETTINGS` documenta dieci impostazioni dell'area, con il loro
valore all'avvio: `history-length`, `minibuffer-width`, `minibuffer-height`,
`*minibuffer-read-function*`, `*completion-read-function*`,
`*completion-filter-function*`, `*completion-invalidated-function*`,
`*command-completion-function*`, `*command-arg-completion-function*` e
`*key-capture-function*`. I loro nomi sono costanti di `editor/settings.rs`,
che le primitive usano al posto delle copie che ciascuna teneva. `help-key`
resta una `defvar` di `help.lisp`: il modulo la definisce e la usa, e il Rust
la legge solo per l'aiuto a metà sequenza, che senza il modulo non c'è. Nei
moduli `completion-window-height`, `completion-columns` e `manpage-path`
passano da `setq` a `defvar` con docstring, e `help-history-length` da
`defconst` a `defvar`. Le 42 docstring delle primitive dell'area e quelle dei
sette moduli seguono il modello; le intestazioni dei moduli rimandano alle
pagine, e i commenti di `common-keymaps.lisp` sul completamento, sui tasti,
sulla ripetizione e sui lavori in background sono diventati rimandi.

Le pagine e le docstring dell'area dicevano cose false, ora corrette:

- nella striscia di completamento le frecce sinistra e destra muovono il
  cursore nel prompt, non la selezione: la muovono su e giù, e Tab sceglie;
- non tutti i candidati si cercano «ovunque nel nome»: i nomi dei file, e
  quelli dei buffer chiesti da un argomento, si completano dall'inizio;
- `C-M-i` non offre solo le parole dei buffer, ma anche i percorsi, le parole
  chiave del modo, i nomi del Lisp e quelli dei buffer, in ogni buffer;
- un tasto si cerca in quattro mappe, non in tre: tra quella transitoria e
  quella del modo c'è quella locale del buffer, che vim dà ai suoi stati;
- la domanda di `M-%` non trattiene i tasti che non conosce: è un'offerta, che
  qualunque altro tasto chiude; una domanda a un tasto come `y-or-n` accetta
  solo y e n, `C-g` compreso; e le mappe transitorie sono di tre tipi, con il
  filtro della striscia;
- `C-h a` cerca nei nomi, con un'espressione regolare, e non nelle
  descrizioni, come dicevano anche `first-steps` e `getting-help`;
- `K` in una pagina del manuale apre la pagina che ha il nome della parola
  sotto il cursore, non la descrizione di una funzione;
- la ricetta per spostare il tasto dell'aiuto non spostava `C-h m` e lasciava
  `C-h` un prefisso, quindi non restituiva Backspace a un terminale che manda
  `C-h`: ora ha le due righe che mancavano;
- `minibuffer-read` e `default-minibuffer-prompt` prendono un sesto
  argomento, la chiave della cronologia, che la firma taceva;
- `register-command` e `command-args` elencavano quattro codici di argomento
  su sette, e `read-key-sequence` taceva la mappa locale; `call-interactively`
  rimandava a `read-command-args`, che non esiste;
- gli esempi di `fuzzy-filter` e `regexp-opt` non si potevano incollare, per
  le virgolette precedute dalla barra, e quello di `define-key` usava
  `lisp-mode`, che non c'è; la docstring di `pending-command` aveva una fila
  di spazi a metà frase;
- l'intestazione di `completion.lisp` diceva che la striscia si scorre con n
  e p: si scorre con `C-n` e `C-p`.

Il passo ha trovato sei difetti del codice, che non corregge; i primi due
sono tra i limiti di `keymaps`:

- `C-h b` elenca i tasti della mappa locale di un buffer, quelli degli stati
  di vim, sotto `Global -- everywhere`, come se valessero ovunque:
  `help--bindings-lines` conosce solo le sorgenti transient, mode e global;
- mentre una domanda è aperta, `key-binding` dà per un tasto che la domanda
  rifiuta ciò che il tasto farebbe senza di lei;
- `install-help-keys` non sposta `C-h m`, che `manpage.lisp` lega da sé, e
  lascia legati i tasti vecchi: la pagina ora dà la ricetta intera;
- nella striscia un candidato con una descrizione lunga riempie la sua
  colonna e si attacca al successivo, senza spazio in mezzo;
- `C-h f` su un comando mostra i codici grezzi dei suoi argomenti: «It asks
  for: fFind file:»;
- `M-:` lega `eval-expression-prompt`, che non è un comando: a
  `M-x eval-expression-prompt` l'editor risponde che il comando non esiste.

#ref-table(
  columns: (1.3cm, 3.0cm, 3.5cm, 3.5cm, 1fr),
  header: ([Passo], [Area], [Pagine], [Primitive (`primitives/`)], [Moduli Lisp e pagine nuove]),
  [2], [avvio, configurazione, diagnostica (fatto)], [`rsedit`, `guide`, `first-steps`, `configuration`], [da `general.rs`: `eval-file`, `log`, `all-logs`, `backtrace`], [`debug`, `commands`; nuova: `diagnostics` (il log nella cache, `*Messages*`, `C-h e`, il backtrace, le righe del watchdog)],
  [3], [scrivere e modificare (fatto)], [`editing`, `rectangles`, `macros`], [`edits`, `verbs`, `region`, `rectangle`, `macros`, `comments`], [`indent`, `electric-pair`, `clipboard`; nuove: `indentation`, `killing`],
  [4], [file, buffer, finestre (fatto)], [`files`, `buffers`, `windows`], [`io`, `buffers`, `windows`, `ui`, `mouse`, `ask`], [`dired`, `buffer-list`; nuove: `dired`, `mouse`; le impostazioni `auto-save` e `watch-files`],
  [5], [cercare e navigare (fatto)], [`searching`, `compile`], [`isearch`, `replace`, `results`, `scan`, `match_data`], [`occur`, `compile`; nuova: `regexps`],
  [6], [prompt, completamento, tasti, aiuto (fatto)], [`completion`, `keymaps`, `getting-help`], [`minibuffer`, `completion`, `help`, `commands`, il resto di `general`], [`minibuffer`, `completion`, `completion-at-point`, `help`, `manpage`, `preview`, `common-keymaps`; nuova: `minibuffer`],
  [7], [modi e aspetto], [`modes`, `themes`, `overlays`, `virtual-text`], [`modes`, `theme`, `overlays`, `virtual_text`], [`rust-mode`, `risp-mode`, `cc-mode`, `theme`],
  [8], [estendere l'editor], [`writing-commands` \ `background`], [`workers`, `shell`], [`shell`; nuova: `shell`],
  [9], [vim], [`vim`], [---], [`vim`],
)

== Fase C --- il codice e i commenti (passi 10--21)

Il passo 10 aggiorna il rapporto sui problemi: ogni scheda verificata sul
codice di quel momento, il campo `stato`, le schede dell'interprete separate
per il repository `risp`. Va prima del codice perché la classificazione dei
commenti (@classificazione) deve sapere quali problemi sono risolti.

Gli altri passi seguono l'ordine delle dipendenze di `lib.rs`, dal basso. Ogni
passo comincia con una proposta breve degli spostamenti --- file da dividere,
unire o rinominare --- che approvi prima della patch. Poi la patch: il
rustdoc del sottosistema secondo le regole, i commenti classificati, il
capitolo del manuale con le sue decisioni e i suoi diagrammi, i commenti dei
test del sottosistema (decisione 8). Il comportamento non cambia: tutti i test
passano senza modifiche, salvo gli spostamenti che seguono quelli del codice.

#ref-table(
  columns: (1.3cm, 7.4cm, 1.6cm, 1fr),
  header: ([Passo], [Parte], [Righe], [Note]),
  [10], [`doc/rapporto-problemi.typ`], [1 244], [80 schede da verificare],
  [11], [`work/`], [2 935], [assorbe `workmanager.typ`],
  [12], [`paths.rs`, `buffer/`], [3 075], [---],
  [13], [`text/`, `input.rs`, `commands.rs`], [2 047], [---],
  [14], [`managers/`], [2 936], [la regola delle coppie con `editor/`],
  [15], [`editor/`], [4 838], [`boot.rs`, `keys.rs`, `frame.rs`],
  [16], [`primitives/`, modifica del testo: `edits`, `verbs`, `region`, `rectangle`, `macros`, `comments`, `match_data`, `scan`], [4 353], [docstring già fatte nella fase B],
  [17], [`primitives/`, file, finestre e lavori: `io`, `buffers`, `windows`, `ui`, `mouse`, `ask`, `shell`, `workers`, `lisp_job`, `args`], [4 995], [`io.rs` è il file più lungo: 1 818 righe],
  [18], [`primitives/`, il resto: `commands`, `completion`, `general`, `help`, `isearch`, `minibuffer`, `modes`, `overlays`, `replace`, `results`, `theme`, `virtual_text`, `mod`], [5 953], [---],
  [19], [`modes/`, `feature/`], [2 896], [---],
  [20], [`ui/`, `src/` (la TUI)], [3 408], [---],
  [21], [`core/lisp/`: intestazioni e commenti], [6 422], [docstring già fatte nella fase B],
)

I criteri con cui si propongono gli spostamenti:

- *La mappa di `lib.rs` è quella vera.* Oggi elenca un modulo `lisp/` che è
  nel crate `risp`. Un test la confronta con le dichiarazioni `mod`.
- *Un concetto per file, e il file ha il nome del concetto.* Un file oltre le
  800 righe circa deve avere un motivo: oggi `primitives/io.rs` (1 818),
  `primitives/edits.rs` (1 567), `ui/windows.rs` (1 437),
  `work/manager.rs` (1 094), `text/search.rs` (952), `primitives/results.rs`
  (936), `src/tui.rs` (924).
- *Una regola sola per le coppie `editor/X` e `managers/X`*: ce ne sono otto
  (`buffers`, `commands`, `kill_yank`, `log`, `macros`, `modes`, `runtime`,
  `windows`). La regola si scrive in `lib.rs` e nel manuale, e vale per tutte.
- *La stessa parola per la stessa cosa*: lavoro, worker e task, per esempio.
- *In un file, prima ciò che è pubblico*, poi gli aiuti privati, nell'ordine
  in cui il lettore li incontra.
- *Un file di test per argomento*, con il nome del modulo che prova.

== Fase D --- il manuale (passo 22)

- `architettura-editor.typ` diventa il manuale per sviluppatori:
  l'introduzione, i diagrammi d'insieme (strati, thread, ordine dei lock), i
  capitoli scritti nella fase C, le ricette, il glossario.
- `workmanager.typ` si toglie, assorbito nel passo 11.
- L'appendice generata delle primitive si toglie: è una copia del registro, e
  `C-h a` la sostituisce.
- Da questo documento escono l'inventario e il piano; restano le regole.
- I PDF si rigenerano.

== Come si consegna ogni passo

Come le patch 1--9. La patch si prepara su una copia di lavoro, si prova con
`git am` su una copia pulita, con l'intera suite di test, e confrontando gli
avvisi di clippy e di `cargo doc` con quelli della base. Arriva con il PDF dei
documenti toccati e con la spiegazione, file per file, di che cosa cambia e
di dove differisce da questo piano.

// ===========================================================================
= Le decisioni <domande>
// ===========================================================================

Le risposte alle domande del passo 0.

+ *Il README.* `man/README.md` si riscrive con il modello delle pagine. Il
  README del progetto diventa una descrizione essenziale secondo le pratiche
  di GitHub, un portfolio sobrio dell'applicazione, con l'elenco dei
  documenti e dei loro destinatari: `README.md` al posto di `README.org`.
+ *La SYNOPSIS come casa dei nomi*, con il rimando automatico in `C-h f`: sì.
+ *Un solo elenco dei moduli*: `build.rs` copia tutta la directory.
+ *I PDF in git*: restano, rigenerati nello stesso commit della sorgente; i
  due orfani si tolgono.
+ *`lisp-fibers.typ`*: lo toglie l'autore.
+ *`workmanager.typ`*: ciò che è ancora valido confluisce nel manuale nel
  passo 11, e il documento si toglie nella fase D.
+ *La storia nei commit*: le patch si applicano con `git am`.
+ *I commenti dei test*: si ripuliscono con le stesse regole insieme al loro
  sottosistema, nella fase C.
+ *Il manuale scritto durante la fase C*: sì.
+ *Le correzioni dell'inventario*: quelle di codice e test nel passo 1,
  quella di `configuration.txt` nel passo 2.
