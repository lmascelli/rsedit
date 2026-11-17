// Il progetto di ciò che manca al modulo Lisp lsp: che cosa farà, sopra quali
// mattoni del nucleo, e che cosa manca ancora al nucleo perché lo si possa
// scrivere. Ciò che è scritto è nel manuale, `<mod-lsp>`; quando il resto c'è,
// passa anch'esso nel manuale, e questo documento si toglie.

#import "style.typ": *

#show: preamble.with(
  title: "Il modulo lsp",
  subtitle: [Progetto di ciò che resta da scrivere: la sincronizzazione a tratti, il completamento, la definizione e le code action],
)

#set text(lang: "it")
#set heading(supplement: [§])
#show table: set par(justify: false)
#show list: set par(justify: false)
#show enum: set par(justify: false)
#set figure(kind: image)
#show figure.caption: set align(start)
#show raw.where(block: true): it => block(breakable: false, it)

// Le figure disegnano il codice in linea piccolo e senza sfondo.
#let canvas = canvas.with(code-size: 7.4pt)

#outline(depth: 2, indent: auto)

// ===========================================================================
= In breve <in-breve>
// ===========================================================================

Il modulo `lsp`, `core/lisp/features/lsp.lisp`, c'è per una parte: il server e
il suo ciclo di vita, la configurazione per modo maggiore, la sincronizzazione
con il testo intero e le segnalazioni. Ciò che fa, e perché, è nel manuale,
`<mod-lsp>`. Questo documento tiene ciò che resta da scrivere, sopra i mattoni
del nucleo che non sanno nulla di LSP: il JSON dell'interprete (R3), il
deposito delle segnalazioni (C5) e il loro disegno (C6), i processi dialoganti
(C7), le posizioni in altre unità e la notifica delle modifiche (C8).

#key[
  *Ciò che resta, in quattro righe.*
  + *La sincronizzazione a tratti* sostituisce quella intera, quando il
    nucleo ha i tre pezzi che le servono (@intero-tratti).
  + *Il completamento è un comando suo*, che offre i candidati quando il
    server risponde, con un pezzo nuovo del nucleo (@completamento-lsp).
  + *La definizione e le code action* non chiedono niente al nucleo
    (@definizione, @code-action).
  + *Le posizioni che escono* passano da una funzione sola, come quelle che
    entrano (@unita).
]

I nomi Lisp che il documento usa per ciò che manca al modulo
(`lsp-complete`, `lsp-find-definition` …) sono proposte; quelli del nucleo
esistono, tranne quelli di @mancano, che sono proposte anche loro e sono
segnati come tali.

// ===========================================================================
= I mattoni <mattoni>
// ===========================================================================

#figure(
  canvas(height: 6.6cm)[
    #node(8.1cm, 0.6cm, 15.8cm, 0.8cm, [*`lsp`*, modulo Lisp: i server, i documenti, segnalazioni, completamento, definizione, code action])

    #node(1.6cm, 2.6cm, 2.9cm, 1.1cm, [*R3* JSON \ `json-parse-string` \ `json-serialize`])
    #node(4.85cm, 2.6cm, 2.9cm, 1.1cm, [*C7* processi \ `process-start` \ `process-send`])
    #node(8.1cm, 2.6cm, 2.9cm, 1.1cm, [*C8* posizioni \ righe e colonne \ `track-changes-end`])
    #node(11.35cm, 2.6cm, 2.9cm, 1.1cm, [*C5* deposito \ `finding-set` \ `findings-for-file`])
    #node(14.6cm, 2.6cm, 2.9cm, 1.1cm, [*C6* schermo \ `findings-mode` \ `M-g f`, `M-g n`])

    #carrow((1.6cm, 1.0cm), (1.6cm, 2.0cm))
    #carrow((4.85cm, 1.0cm), (4.85cm, 2.0cm))
    #carrow((8.1cm, 1.0cm), (8.1cm, 2.0cm))
    #carrow((11.35cm, 1.0cm), (11.35cm, 2.0cm))

    #group(0.0cm, 4.1cm, 16.2cm, 2.2cm, [già nel nucleo e nei moduli, usati come sono])
    #node(1.7cm, 5.2cm, 2.9cm, 1.1cm, [`with-directory` (C2) \ la radice come \ directory di lavoro], fill: paper)
    #node(4.9cm, 5.2cm, 2.9cm, 1.1cm, [`project-root` (M1) \ la radice di \ un file], fill: paper)
    #node(8.1cm, 5.2cm, 2.9cm, 1.1cm, [`results-put` \ una lista per \ `M-g n`], fill: paper)
    #node(11.3cm, 5.2cm, 2.9cm, 1.1cm, [`minibuffer-read` \ una scelta \ tra titoli], fill: paper)
    #node(14.5cm, 5.2cm, 2.9cm, 1.1cm, [gli hook dei \ buffer e dei \ comandi], fill: paper)
  ],
  caption: [
    Il modulo e ciò che usa. Le frecce dicono «chiama»: il modulo non è
    chiamato da nessun mattone, e non chiama C6, che legge da sé il deposito
    a ogni fotografia. Ciascuno dei mattoni ha già un cliente che
    non è LSP (`compile` per il deposito, un filtro qualunque per i processi,
    vim per la notifica delle modifiche).
  ],
)

#ref-table(
  columns: (2.4cm, 5.6cm, 1fr),
  header: ([Mattone], [Che cosa dà], [Che cosa ne fa il modulo]),
  [R3 JSON], [`json-parse-string`: oggetti in mappe con chiavi stringa, array in vettori, `true` in `t`, `false` e `null` nei simboli `false` e `null`, numeri in float. `json-serialize`: il contrario, con le chiavi ordinate; rifiuta nil e le liste.], [Legge ogni corpo e scrive ogni messaggio. Un campo assente si omette, un array vuoto è `[]`, un booleano falso è `false`: nil non entra mai in un messaggio, perché `json-serialize` lo rifiuterebbe. Gli id delle richieste sono interi piccoli, esatti come float.],
  [C7 processi], [`process-start` con la forma `messages`: la cornice `Content-Length` letta e scritta in Rust; consegne tra un comando e l'altro, una chiamata `(FUNCTION NOME TIPO VALORE)` ciascuna, `exit` sempre per ultima; `process-send`, che non aspetta e risponde nil quando la coda dell'ingresso è piena; `process-stop`, `process-status`.], [Un processo per server. La funzione di consegna smista le risposte, le richieste del server e le notifiche; lo stderr va nel registro del server. Ciò che `process-send` non accetta resta in una coda del modulo, nell'ordine, e si riprova (manuale, `<mod-lsp>`).],
  [C8 posizioni], [`position-to-line-column` e `line-column-to-position`, con l'unità `char`, `utf-16` o `utf-8`; righe da 1. `track-changes-begin` e `track-changes-end`: che cosa è cambiato da quando si guarda, come `(START REMOVED INSERTED)`.], [Converte ogni posizione che entra o esce (@unita). Sa che cosa mandare a `didChange` (@intero-tratti), una volta che il nucleo ha i pezzi che mancano (@mancano).],
  [C5 deposito], [`finding-set PRODUCER FILE ENTRIES &optional UNIT` sostituisce in blocco; `finding-clear`; le domande per file, produttore, directory. Posizioni del produttore e marcatori quando il file è aperto.], [Un produttore per server. Ogni `publishDiagnostics` diventa un `finding-set` per quel file (manuale, `<mod-lsp>`).],
  [C6 schermo], [`findings-mode`, acceso ovunque dal modulo `findings`: intervallo, segno, conteggio, messaggio al punto; `M-g f` e `M-g n`.], [Niente: le segnalazioni di un server si vedono come quelle di `compile`.],
)

// ===========================================================================
= La sincronizzazione a tratti <intero-tratti>
// ===========================================================================

Oggi ogni svuotamento che trova cambiato il `buffer-modified-tick` di un
documento manda il testo intero (manuale, `<mod-lsp>`). Il modo a tratti manda
invece un `contentChanges` solo, con l'intervallo nel testo di prima e il testo
nuovo, e prende il posto di `lsp--changed` senza cambiare niente altro del
modulo. Ha bisogno di N2, N3 e N4 (@mancano).

Il modo a tratti legge ciò che è cambiato con `track-changes-end`, sotto una
chiave sua (N2), e ricomincia subito a guardare. La risposta `(START REMOVED
INSERTED)` è in caratteri sul testo di adesso. L'inizio dell'intervallo è lo
stesso nel testo di prima e in quello di adesso, perché davanti a START niente
è cambiato: `position-to-line-column` lo traduce. La fine nel testo di prima
non si può leggere nel buffer, dove quel testo non c'è più: si ricava
dall'inizio e dal testo tolto, che `track-changes-end` oggi non dà (N3),
misurato in righe e colonne nell'unità del server (N4). Quando il testo tolto
non si sa --- un undo che è uscito dal tratto guardato, un file riletto da
disco --- il modulo manda il testo intero: è ciò che `track-changes` dice già
in quei casi, facendo del tratto intero il cambiamento.

// ===========================================================================
= Le posizioni che escono <unita>
// ===========================================================================

Le posizioni che entrano passano già da una funzione sola, `lsp--position`,
che legge una posizione LSP nell'unità del server come un offset del buffer
corrente (manuale, `<mod-lsp>`). Quelle che escono --- il punto di un
completamento o di una definizione, l'intervallo di una code action ---
passeranno dalla funzione del verso contrario, sola anch'essa: da un offset del
buffer corrente a una posizione LSP, con `position-to-line-column`
nell'unità del server e la riga meno uno. Altrove non si converte, come per le
espressioni regolari `char_to_byte` e `to_match` (manuale, `<testo>`).

// ===========================================================================
= Le funzioni che restano <funzioni>
// ===========================================================================

Per ciascuna: che cosa chiede al server, quali mattoni usa, che cosa manca. Ciascuna aggiunge alle
capacità del client (manuale, `<mod-lsp>`) solo ciò che onora, perché un server
che crede il client capace di qualcosa glielo manda: il completamento
`snippetSupport` falso; le code action `codeAction.resolveSupport` per la
proprietà `edit`, `resourceOperations` vuoto nelle modifiche e
`workspace.applyEdit`, che oggi il client rifiuta.

== Il completamento <completamento-lsp>

`textDocument/completion` alla posizione del punto risponde un elenco di voci,
o una `CompletionList` che dice anche se l'elenco è parziale. Di ogni voce
servono il testo da mettere (`textEdit.newText`, o `insertText`, o `label`), la
descrizione (`detail`), il tratto che sostituisce (`textEdit.range`) e le
modifiche in più (`additionalTextEdits`, come un `use` aggiunto in testa).

Le fonti di `completion-at-point` (manuale, `<completamento>`) rispondono
subito, dentro il comando: `(START END CANDIDATES)` o nil. Il server risponde
dopo, tra un comando e l'altro, e una fonte non può aspettarlo: la risposta è
consegnata solo quando il comando è finito (C7), e la fonte aspetterebbe fino
alla scadenza. Serve un modo di offrire i candidati più tardi, che il nucleo
non ha (N6): una primitiva che faccia, sul buffer corrente, ciò che
`completion-at-point` fa dopo aver sentito le fonti --- il filtro, un
candidato solo messo al posto del testo, più d'uno al presentatore --- e che,
a scelta fatta, chiami una funzione con il candidato scelto, per le
`additionalTextEdits`.

Con N6 il completamento è un comando, `lsp-complete`, legato nella keymap di
`lsp-mode` al tasto di `completion-at-point`. Svuota, manda la richiesta, e
alla risposta, se il buffer, il punto e la versione sono ancora quelli,
offre i candidati con N6; il tratto è la `range` della prima voce che ne ha
una, o il simbolo al punto (`bounds-of-thing-at-point`). Un completamento
richiesto mentre ce n'è uno in corso prende il suo posto (manuale, `<mod-lsp>`). Il
completamento automatico dopo un carattere di innesco
(`triggerCharacters`, da `post-self-insert-hook`) viene dopo, e spento.

Usa C7, R3, C8 per il tratto, il presentatore e il filtro che ci sono. Manca
N6.

== La definizione <definizione>

`textDocument/definition` alla posizione del punto risponde una posizione, un
elenco di posizioni, un elenco di `LocationLink`, o null.

- *Una:* il modulo ricorda dove si era --- buffer e punto, su una pila sua ---
  apre il file con `find-file`, che torna con il buffer corrente e il testo
  letto, traduce la posizione nel buffer aperto con le funzioni di @unita e ci
  porta il punto. `lsp-go-back` toglie dalla pila e torna.
- *Più d'una:* una lista di risultati (manuale, `<risultati>`) con
  `results-put`, una voce `("file" PERCORSO RIGA COLONNA)` per posizione, che
  `M-g n` percorre come quella di un `grep`. La colonna di un file non aperto è
  quella del server, come per le segnalazioni.
- *Null:* un messaggio.

`textDocument/declaration`, `typeDefinition`, `implementation` e `references`
hanno la stessa risposta, e passano dalla stessa funzione: `references` dà
quasi sempre una lista.

Usa C7, R3, C8, e `find-file` e le liste di risultati che ci sono. Al nucleo
non manca niente. Manca, ma fuori dal nucleo, una pila dei posti da cui si è
saltati che non sia del solo `lsp`: l'editor non ha un anello dei mark. La
prima versione la tiene nel modulo; quando un secondo modulo ne vorrà una ---
una ricerca dei tag, un `grep` che salta --- diventerà un modulo Lisp suo.

== Le code action <code-action>

`textDocument/codeAction` per un intervallo --- la regione se è attiva, il
punto altrimenti --- risponde le azioni possibili: comandi da far eseguire al
server, o azioni con una modifica (`edit`), un comando, o tutti e due.

- *Il contesto.* La richiesta porta le segnalazioni del server che toccano
  l'intervallo, com'erano arrivate. Il modulo le sceglie con
  `findings-for-file`, che le dà al posto dei marcatori, cioè nel testo di
  adesso: tra quelle con il `:producer` del server, il posto nell'elenco di
  quelle che toccano l'intervallo dice quali grezze mandare, perché il
  deposito tiene le segnalazioni di un produttore nell'ordine in cui le ha
  ricevute.
- *La scelta.* I titoli in un prompt, `minibuffer-read` con una funzione dei
  candidati come `project-switch`, con prima quelle `isPreferred` e senza
  quelle `disabled`. Un'azione senza `edit`, a un server che dichiara
  `resolveProvider`, si chiede intera con `codeAction/resolve` prima di
  applicarla: `rust-analyzer` manda così le sue.
- *La modifica.* Un `WorkspaceEdit` dice, per URI, una lista di sostituzioni
  (`TextEdit`: intervallo e testo nuovo), con la versione del documento se
  viene da `documentChanges`. Per ogni file: il buffer che lo visita, o
  `find-file`; se la versione c'è e non è quella del documento, si rifiuta
  tutta la modifica, perché è stata calcolata su un altro testo. Le
  sostituzioni si traducono in offset tutte prima di cominciare, sul testo
  com'è; si ordinano, si rifiuta la modifica se due si sovrappongono, e
  `replace-regions` (N7) le applica dall'ultima alla prima, come fanno i
  commenti (manuale, `<modifiche-multiple>`), in un passo di undo per buffer. I file
  toccati restano da salvare. Poi lo svuotamento manda al server ciò che ha
  scritto.
- *Il comando.* `workspace/executeCommand` con il comando e i suoi argomenti,
  così come sono arrivati. Il server, di solito, risponde con una richiesta
  `workspace/applyEdit`, che passa dalla modifica appena descritta.

Usa C7, R3, C8, C5 per il contesto, `minibuffer-read`, e N7,
`replace-regions`, che applica dall'ultima alla prima una lista di
sostituzioni tenendo il punto al suo posto, con lo stesso `apply` dei
commenti. Al nucleo non manca niente.

// ===========================================================================
= Che cosa manca nel nucleo <mancano>
// ===========================================================================

Ogni pezzo è agnostico: il nome e la pagina parlano di buffer, modifiche,
salvataggi e candidati, mai di server o di progetti. Ognuno ha già, o avrà
subito, un cliente che non è `lsp`.

#ref-table(
  columns: (0.9cm, 4.2cm, 1fr, 2.4cm),
  header: ([], [Pezzo], [Che cosa e perché], [Serve a]),
  [N2], [più osservatori: `track-changes-begin` e `track-changes-end` con una `KEY` facoltativa], [Oggi `Buffer::touched` è un `Option<Touched>` e `track-changes-begin` lo rimpiazza, dimenticando ciò che si guardava prima (V1: no, non regge più osservatori). Vim guarda una sessione di Insert; il modulo, nello stesso buffer, gliela toglierebbe, e `.` ripeterebbe la cosa sbagliata. La modifica minima: una piccola lista di `(KEY, Touched)` per buffer, le porte che avvisano ognuno, nil come chiave di chi non ne dà una, così vim non cambia.], [sincronizzazione a tratti],
  [N3], [il testo tolto in `track-changes-end`], [Un quarto elemento, `(START REMOVED INSERTED REMOVED-TEXT)`, nil quando non si sa (il `before` di `Touched` già perso da un undo o da una rilettura). `Touched` tiene già quel testo: è la sua parte fra il prefisso e il suffisso comuni. Serve a chiunque debba ricostruire il testo di prima: un registro delle modifiche, un confronto.], [sincronizzazione a tratti],
  [N4], [`(string-line-column STRING &optional UNIT)`], [Quante righe nuove ci sono in STRING, e la colonna della sua fine nell'unità di C8: la misura che `text/positions.rs` sa fare su un testo, fatta su una stringa. In Lisp costerebbe un giro per carattere, e una regione di un mega tolta un mega di giri.], [sincronizzazione a tratti],
  [N6], [`(completion-offer START END CANDIDATES &optional ON-CHOSEN)`], [Offre CANDIDATES per START..END del buffer corrente come `completion-at-point` offre ciò che le fonti hanno dato: filtro, uno solo al posto del testo, più d'uno al presentatore; poi chiama ON-CHOSEN con il candidato messo. Chi chiama controlla prima che il buffer e il punto siano quelli. Serve a ogni fonte che risponde più tardi: una REPL, una ricerca su disco.], [completamento],
)

#ref-table(
  columns: (0.9cm, 4.2cm, 1fr),
  header: ([], [Facoltativo], [Perché non ora]),
  [N10], [`after-close-hook` che dice quale buffer], [Il modulo confronta con `all-buffer-names`.],
)

L'ordine: la definizione e le code action non aspettano niente; N6 per il
completamento; N2, N3 e N4 insieme per la sincronizzazione a tratti.

// ===========================================================================
= Le decisioni <decisioni>
// ===========================================================================

+ *Il completamento è un comando suo, non una fonte.* Una fonte risponde
  dentro il comando, e il server dopo; finché il nucleo non ha N6 non c'è un
  completamento, piuttosto di uno che aspetta. Scartato: una fonte che
  risponde dall'ultima risposta ricordata, che offre al primo tasto i
  candidati della parola di prima.
