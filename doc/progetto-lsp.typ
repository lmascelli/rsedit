// Il progetto del modulo Lisp lsp: che cosa farà, sopra quali mattoni del
// nucleo, e che cosa manca ancora al nucleo perché lo si possa scrivere.
// Quando il modulo c'è, il progetto diventa la sua sezione del manuale e le
// sue decisioni, e questo documento si toglie.

#import "style.typ": *

#show: preamble.with(
  title: "Il modulo lsp",
  subtitle: [Progetto di un modulo da scrivere: il server, i documenti, e le quattro funzioni sopra i mattoni del nucleo],
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

Un server di un linguaggio è un programma che resta acceso accanto all'editor,
legge i file di un progetto e risponde a domande sul codice: che cosa non va,
come si può completare la parola al punto, dove è definito un nome, che cosa
si può fare per correggere un errore. Il protocollo è LSP: messaggi JSON-RPC,
ciascuno preceduto da un'intestazione `Content-Length`.

Il modulo `lsp` sarà un modulo Lisp, `core/lisp/features/lsp.lisp`, che
orchestra mattoni del nucleo che non sanno nulla di LSP: il JSON
dell'interprete (R3), il deposito delle segnalazioni (C5) e il loro disegno
(C6), i processi dialoganti (C7), le posizioni in altre unità e la notifica
delle modifiche (C8). Come `project`, è sostituibile da un plugin: niente del
nucleo lo nomina.

#key[
  *Il progetto, in sei righe.*
  + *Un server per coppia (configurazione, radice)*, avviato al primo buffer
    che lo chiede, con `process-start` nella forma `messages`.
  + *La configurazione è una lista per modo maggiore*, `lsp-servers`, e il
    modo minore `lsp-mode` la usa dove `minor-mode-enable` lo accende.
  + *Il modulo tiene la sua copia dello stato del protocollo* --- richieste in
    corso, versioni dei documenti, segnalazioni grezze --- e il nucleo solo
    ciò che vale per chiunque: testo, segnalazioni, processi.
  + *Le modifiche si mandano prima di ogni altra cosa*: dopo ogni comando,
    all'inizio di ogni consegna del server, prima di ogni richiesta.
  + *Le colonne si contano in caratteri quando il server lo accetta*
    (`utf-32`), in UTF-16 altrimenti; la conversione sta in due funzioni sole.
  + *Al nucleo mancano sei pezzi piccoli e agnostici* (@mancano), due dei quali
    servono già alla prima versione.
]

Lo stato: niente di questo è scritto. I nomi Lisp che il documento usa per il
modulo (`lsp-servers`, `lsp-mode`, `lsp-find-definition` …) sono proposte;
quelli del nucleo esistono, tranne quelli di @mancano, che sono proposte anche
loro e sono segnati come tali.

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
  [C7 processi], [`process-start` con la forma `messages`: la cornice `Content-Length` letta e scritta in Rust; consegne tra un comando e l'altro, una chiamata `(FUNCTION NOME TIPO VALORE)` ciascuna, `exit` sempre per ultima; `process-send`, che non aspetta e risponde nil quando la coda dell'ingresso è piena; `process-stop`, `process-status`.], [Un processo per server. La funzione di consegna smista le risposte, le richieste del server e le notifiche; lo stderr va nel registro del server. Ciò che `process-send` non accetta resta in una coda del modulo, nell'ordine, e si riprova (@richieste).],
  [C8 posizioni], [`position-to-line-column` e `line-column-to-position`, con l'unità `char`, `utf-16` o `utf-8`; righe da 1. `track-changes-begin` e `track-changes-end`: che cosa è cambiato da quando si guarda, come `(START REMOVED INSERTED)`.], [Converte ogni posizione che entra o esce (@unita). Sa che cosa mandare a `didChange` (@sincronizzazione), una volta che il nucleo ha i pezzi che mancano (@mancano).],
  [C5 deposito], [`finding-set PRODUCER FILE ENTRIES` sostituisce in blocco; `finding-clear`; le domande per file, produttore, directory. Posizioni del produttore e marcatori quando il file è aperto.], [Un produttore per server. Ogni `publishDiagnostics` diventa un `finding-set` per quel file (@segnalazioni-lsp).],
  [C6 schermo], [`findings-mode`, acceso ovunque dal modulo `findings`: intervallo, segno, conteggio, messaggio al punto; `M-g f` e `M-g n`.], [Niente: le segnalazioni di un server si vedono come quelle di `compile`.],
)

// ===========================================================================
= Il ciclo di vita del server <ciclo-server>
// ===========================================================================

== Chi è un server <chi-server>

Un server è una voce di `lsp-servers` (@configurazione) che gira in una
radice. La chiave è la coppia (nome della voce, radice): due crate dello stesso
repository hanno un server solo, due repository due. Il nome del processo per
C7 e il nome del produttore per C5 sono la stessa stringa, `lsp NOME RADICE`,
così `work-list`, `process-status` e `findings-of-producer` parlano dello
stesso server.

La radice la chiede `lsp-root-function`, una variabile che vale
`project-root`: il modulo `project` sa già trovarla, con `.git` che vince e le
radici dichiarate a mano, e un plugin che la cerca in un altro modo la
sostituisce senza toccare `lsp`. `project-root` risponde una directory, nil o
`timeout`. Con nil la radice è la directory del file. Con `timeout` il
documento resta in attesa e la domanda si rifà al comando seguente, quando la
risalita ha la sua risposta ricordata.

Il modulo tiene, per ogni server:

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Campo], [Contenuto]),
  [stato], [`starting`, `ready`, `stopping` o `dead`.],
  [unità], [l'unità delle colonne che il server ha scelto, già tradotta in quella di C8: `char` o `utf-16` (@unita).],
  [capacità], [la mappa `capabilities` della risposta a `initialize`, così com'è.],
  [richieste], [una mappa dall'id alla richiesta in corso: metodo, funzione da chiamare con la risposta, buffer, versione del documento, ora di invio.],
  [coda d'uscita], [i messaggi che `process-send` non ha ancora accettato, nell'ordine.],
  [documenti], [i buffer aperti presso il server (@sincronizzazione).],
  [segnalazioni grezze], [per file, l'ultimo elenco di `publishDiagnostics` così come è arrivato, per le code action (@code-action).],
  [riavvii], [le ore delle ultime cadute, per il limite dei riavvii.],
)

== L'avvio <avvio-server>

Il primo buffer che accende `lsp-mode` in una radice senza server lo fa
partire. `process-start` gira dentro `with-directory` sulla radice (C2), con il
programma e gli argomenti della voce, senza shell, e la forma `messages`.

#figure(
  canvas(height: 8.4cm)[
    #lane(1.5cm, 0cm, 8.2cm, [buffer, comandi], w: 2.8cm)
    #lane(5.6cm, 0cm, 8.2cm, [modulo `lsp`])
    #lane(9.8cm, 0cm, 8.2cm, [processo (C7)], fill: accent.lighten(85%))
    #lane(14.2cm, 0cm, 8.2cm, [server], fill: accent.lighten(85%))

    #msg(1.5cm, 5.6cm, 1.2cm, [`lsp-mode` acceso: file, radice])
    #msg(5.6cm, 9.8cm, 1.7cm, [`process-start` … `'messages`])
    #msg(5.6cm, 14.2cm, 2.2cm, [`initialize`, `positionEncodings`])
    #msg(14.2cm, 5.6cm, 2.75cm, [risposta: capacità, unità], dash: "dashed")
    #msg(5.6cm, 14.2cm, 3.3cm, [`initialized`; `didOpen` di chi attende])
    #self-msg(1.5cm, 3.9cm, [un comando scrive])
    #msg(1.5cm, 5.6cm, 4.75cm, [`post-command-hook`])
    #msg(5.6cm, 14.2cm, 5.25cm, [`didChange`, versione + 1])
    #msg(14.2cm, 5.6cm, 5.8cm, [`publishDiagnostics`, tra due comandi], dash: "dashed")
    #self-msg(5.6cm, 6.15cm, [`finding-set`])
    #msg(5.6cm, 14.2cm, 7.2cm, [`shutdown`, poi `exit`])
    #msg(9.8cm, 5.6cm, 7.75cm, [consegna `exit`], dash: "dashed")
  ],
  caption: [
    La vita di un server, dall'avvio all'arresto. Tutto ciò che arriva dal
    server arriva tra un comando e l'altro, come le callback del lavoro di
    sfondo; il modulo non aspetta mai una risposta dentro un comando.
  ],
)

`initialize` porta `processId` null (l'editor non dice il suo numero di
processo, e il server non ne ha bisogno), `rootUri` e un solo
`workspaceFolders`, la radice, e le capacità del client: solo quelle che il
modulo sa onorare (@capacita). Finché la risposta non arriva il server è
`starting`: i documenti che si aprono aspettano, e le richieste dei comandi non
partono, con un messaggio. Alla risposta il modulo legge l'unità e le
capacità, manda `initialized`, poi `didOpen` per i documenti in attesa, e il
server è `ready`.

== Richieste, risposte e messaggi del server <richieste>

Ogni consegna `output` di C7 è un corpo intero. Il modulo lo legge con
`json-parse-string` e guarda due campi: con `id` e senza `method` è una
risposta, con `method` e `id` una richiesta del server, con `method` e senza
`id` una notifica.

- *Una risposta* si cerca per id tra le richieste in corso, e se ne toglie. Un
  `error` va nel registro e, se un comando l'aveva chiesta, nell'area
  messaggi. Un `result` va alla funzione della richiesta solo se ciò su cui
  era stata chiesta c'è ancora: il buffer vive, e la versione del documento è
  quella dell'invio. Altrimenti si scarta: è la risposta a una domanda che
  nessuno fa più.
- *Una richiesta nuova dello stesso tipo sullo stesso documento* ---
  completamento, code action --- prende il posto della vecchia, che si toglie
  dalle richieste in corso e per cui si manda `$/cancelRequest`.
- *Le scadenze* le guarda un lavoro `background-call` che tiene solo il tempo,
  come il resoconto di `compile`, acceso finché c'è una richiesta in corso: una
  richiesta oltre `lsp-request-timeout` secondi si toglie, con un messaggio.
- *L'uscita*: `process-send` non aspetta mai; quando risponde nil il messaggio
  resta nella coda d'uscita del server, e la coda si svuota, nell'ordine, a
  ogni invio e a ogni consegna. Un messaggio non si scavalca: un `didChange`
  perso lascerebbe il server con un testo diverso, e nessuno se ne
  accorgerebbe. Una coda d'uscita oltre un limite vuol dire un server che non
  legge più: lo si ferma, come una caduta.

Il server fa domande anche lui, e a ognuna si risponde, anche con un errore:
un server che aspetta una risposta può fermarsi. Le notifiche che il modulo non
conosce si ignorano.

#ref-table(
  columns: (5.6cm, 1fr),
  header: ([Dal server], [Risposta]),
  [`workspace/configuration`], [per ogni voce, la sezione chiesta di `:settings` (@configurazione), o null.],
  [`workspace/applyEdit`], [applica la modifica come una code action (@code-action), e risponde `applied` vero o `false`.],
  [`client/registerCapability`, `client/unregisterCapability`], [null: il client non dichiara la registrazione dinamica, e le registrazioni non cambiano niente.],
  [`window/workDoneProgress/create`], [null; le `$/progress` che seguono finiscono nel registro, e la loro fine nell'area messaggi.],
  [`window/showMessageRequest`], [null nella prima versione: il messaggio si mostra, senza le scelte.],
  [`window/showMessage`, `window/logMessage`], [notifiche: l'area messaggi e il registro.],
  [ogni altra richiesta], [l'errore `-32601`, metodo sconosciuto.],
)

Il registro di un server è un buffer, `*lsp NOME*`, con lo stderr e i
messaggi del server, troncato dall'inizio oltre una misura: serve a chi
indaga, e non deve crescere quanto la sessione.

== L'arresto, la caduta, il riavvio <arresto-server>

`lsp-shutdown` manda `shutdown`; alla risposta manda `exit` e chiude
l'ingresso (`process-close-input`). Se dopo due secondi la consegna `exit` non
è arrivata, `process-stop`. Alla consegna `exit` il modulo toglie le
segnalazioni del server (`finding-clear` del produttore), dimentica le sue
richieste e le sue segnalazioni grezze, e mette i suoi documenti in attesa.

Una consegna `exit` che nessuno ha chiesto è una caduta. Dopo una caduta il
server si riavvia da sé, e i documenti in attesa si riaprono presso il nuovo,
al più `lsp-restart-limit` volte in tre minuti; oltre, resta `dead`, con un
messaggio, finché `lsp-restart` non lo riavvia.

Chiudere l'ultimo buffer di un server non lo ferma: riaprire un file del
progetto costerebbe di nuovo l'indicizzazione. All'uscita dell'editor non c'è
un hook (@mancano): `kill_left_over` ferma i gruppi dei processi ancora vivi,
e il server riceve `TERM` senza `shutdown`. I server lo sopportano, perché è
ciò che succede quando un editor cade.

// ===========================================================================
= La configurazione per modo maggiore <configurazione>
// ===========================================================================

`lsp-servers` è una lista, dichiarata con `defvar` e la sua docstring, con una
voce per modo maggiore:

```lisp
(MODE NOME (PROGRAMMA ARGOMENTI...) . PROPRIETÀ)
```

#ref-table(
  columns: (4.4cm, 1fr),
  header: ([Proprietà], [Che cosa dice]),
  [`:language`], [il `languageId` di `didOpen`; manca: il nome del modo senza `-mode`, che va per `rust`, `c`, `cpp`, `python`.],
  [`:initialization-options`], [una mappa, mandata così com'è in `initialize`.],
  [`:settings`], [una mappa dalle sezioni, `"rust-analyzer"`, alle loro impostazioni, per `workspace/configuration`.],
  [`:root`], [una funzione che sostituisce `lsp-root-function` per questa voce.],
)

Il modulo porta voci per i linguaggi che l'editor colora e per cui c'è un
server diffuso: `rust-mode` con `rust-analyzer`, `c-mode` e `cpp-mode` con
`clangd`, `python-mode` con `pylsp`. Ma non accende nulla: un server è un
programma che l'utente deve avere, e un editor che ne lancia uno mancante a
ogni file aperto dà un errore a ogni file aperto. L'accensione è dell'utente,
con il meccanismo dei modi minori (manuale, `<modi-minori>`):

```lisp
(require 'lsp)
(minor-mode-enable 'lsp-mode 'rust-mode)        ; ogni file Rust
(setq lsp-servers
      (cons '(python-mode "pyright" ("pyright-langserver" "--stdio"))
            lsp-servers))                       ; un altro server per Python
```

Di due voci per lo stesso modo vince la prima, come nei `compilation-patterns`:
l'esempio mette `pyright` davanti a `pylsp`. `lsp-mode` acceso in un buffer
che non visita un file, o il cui modo non ha voce, si spegne da sé con un
messaggio. Un modo ha un server solo: due server
sullo stesso buffer --- un linter accanto al server del linguaggio ---
vorrebbero due documenti, due versioni e la fusione delle risposte, e la prima
versione non li tiene.

Le impostazioni del modulo, ciascuna con `defvar`: `lsp-servers`,
`lsp-root-function`, `lsp-request-timeout`, `lsp-restart-limit`,
`lsp-log-limit`. I tasti stanno nella keymap di `lsp-mode`: `M-.`
`lsp-find-definition`, `M-,` `lsp-go-back`, `C-c a` `lsp-code-action`, e il
completamento sul tasto di `completion-at-point` (@completamento-lsp).

// ===========================================================================
= La sincronizzazione dei documenti <sincronizzazione>
// ===========================================================================

Un documento è un buffer con `lsp-mode` che visita un file, aperto presso il
suo server. Il modulo ne tiene il nome del buffer, il file, l'URI, la versione
--- un intero che parte da 1 e cresce a ogni `didChange` --- e se il server
l'ha già ricevuto.

#ref-table(
  columns: (3.4cm, 4.6cm, 1fr),
  header: ([Notifica], [Quando], [Che cosa]),
  [`didOpen`], [`lsp-mode` si accende in un buffer (`minor-mode-hook`), o il server diventa `ready`], [il testo intero, `buffer-string`, con `languageId` e versione 1; da qui il modulo guarda le modifiche del buffer.],
  [`didChange`], [lo svuotamento, sotto], [ciò che è cambiato dall'ultimo, intero o a tratti secondo il server e il nucleo.],
  [`didSave`], [dopo un salvataggio: `after-save-hook`, che manca (@mancano)], [il server rifà i controlli che fa al salvataggio: `rust-analyzer` lancia lì `cargo check`.],
  [`didClose`], [`lsp-mode` si spegne; `after-close-hook`; `after-set-visited-file-name-hook` per il nome vecchio, seguito da un `didOpen` per il nuovo], [l'URI; il documento si dimentica.],
)

`after-close-hook` gira quando il buffer non c'è più, e non dice quale fosse:
il modulo confronta i suoi documenti con `buffer-list` e chiude quelli il cui
buffer è sparito. Costa una lista dei buffer a ogni chiusura, ed evita un pezzo
nuovo nel nucleo.

== Lo svuotamento <svuotamento>

Il modulo non manda una modifica per ogni tasto, ma una per documento a ogni
svuotamento, con tutto ciò che è cambiato dall'ultimo. Si svuota in tre
momenti:

+ *dopo ogni comando*, da `post-command-hook` per ogni modo; un comando
  fallito non lo chiama, e ciò che ha scritto parte al momento seguente;
+ *all'inizio di ogni consegna del server*, perché una callback di un altro
  modulo, tra due comandi, può aver scritto in un documento;
+ *prima di ogni invio di una richiesta*, perché le posizioni della richiesta
  sono quelle del testo di adesso, e il server deve averlo.

Con il terzo momento vale una regola sola: il server ha sempre ricevuto il
testo su cui gli si chiede qualcosa. Con il secondo, una risposta arriva quando
la versione del documento dice già ogni modifica fatta, e il confronto delle
versioni basta a scartare quelle vecchie (@richieste). Il modulo stesso scrive
nei documenti --- una code action, un completamento --- e svuota alla fine.

Svuotare un documento è una domanda al suo buffer, dentro
`with-current-buffer`: un buffer che nessuno ha toccato risponde subito. Il
costo è una domanda per documento aperto dopo ogni comando.

== Intero o a tratti <intero-tratti>

#ref-table(
  columns: (2.8cm, 5.0cm, 1fr),
  header: ([Modo], [Che cosa serve dal nucleo], [Che cosa manda]),
  [Intero], [sapere se il testo è cambiato: `buffer-modified-tick` (@mancano, N1)], [`buffer-string` intero, a ogni svuotamento che trova la versione del buffer cambiata. Costa la misura del file a ogni comando che scrive.],
  [A tratti], [N2, N3, N4 (@mancano)], [un `contentChanges` solo, con l'intervallo nel testo di prima e il testo nuovo.],
)

La prima versione manda il testo intero, che chiede un solo pezzo nuovo e non
può sbagliare. Il modo a tratti viene dopo, perché un intervallo sbagliato
lascia il server con un testo diverso senza che nessuno se ne accorga, e ha
bisogno di tre pezzi.

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

== Le unità delle colonne <unita>

LSP conta le colonne in unità UTF-16, salvo che client e server si accordino
su un'altra: il client ne offre in `general.positionEncodings`, il server ne
sceglie una e la dice in `capabilities.positionEncoding`. Il modulo offre
`utf-32` e poi `utf-16`. `utf-32` conta i caratteri, come l'editor
(`Unit::Char`): nessuna conversione, e le colonne dei file non aperti sono
giuste anche senza il loro testo. `utf-16` lo deve accettare ogni server, ed è
`Unit::Utf16` di C8. `utf-8` non si offre: in una lista di risultati o in una
segnalazione di un file non aperto una colonna in byte sbaglia a ogni lettera
accentata, mentre una in UTF-16 sbaglia solo dopo un carattere fuori dal piano
di base, come una faccina.

Ogni posizione che entra o esce passa da due funzioni del modulo, una per
verso, come `char_to_byte` e `to_match` per le espressioni regolari (manuale,
`<testo>`): da una posizione LSP a un offset del buffer corrente, con
`line-column-to-position` e la riga più uno; e il contrario, con
`position-to-line-column` e la riga meno uno. Altrove non si converte.

== Le capacità del client <capacita>

Il client dichiara solo ciò che il modulo sa fare, perché un server che crede
il client capace di qualcosa glielo manda: niente snippet nel completamento
(`snippetSupport` falso), niente operazioni sui file nelle modifiche
(`resourceOperations` vuoto), niente registrazione dinamica, `didSave` solo
quando il nucleo ha `after-save-hook`, `codeAction.resolveSupport` per la
proprietà `edit`, `publishDiagnostics.versionSupport`.

// ===========================================================================
= Le quattro funzioni <funzioni>
// ===========================================================================

Per ciascuna: che cosa chiede al server, quali mattoni usa, che cosa manca.

== Le segnalazioni <segnalazioni-lsp>

`textDocument/publishDiagnostics` arriva quando il server vuole, per un URI,
con l'elenco intero delle segnalazioni di quel file: la forma esatta di
`finding-set`, che sostituisce in blocco. Il modulo fa una voce per
segnalazione: `:start` e `:end` dalla `range`, `:severity` da 1--4 in `error`,
`warning`, `info`, `hint` (una gravità assente è `error`: il protocollo lascia
la scelta al client), `:message`, `:code` se c'è. Un elenco vuoto toglie le
segnalazioni di quel file, come `finding-set` con una lista vuota.

- *Le posizioni.* Il deposito vuole riga da 1 e colonna da 0 in caratteri. Con
  `utf-32` è solo la riga più uno. Con `utf-16`, in un file aperto, le due
  funzioni di @unita traducono nel buffer; in un file non aperto la colonna
  resta quella del server, giusta salvo dopo un carattere fuori dal piano di
  base.
- *La versione.* Una `publishDiagnostics` con una `version` diversa da quella
  del documento parla di un testo che non c'è più, e il deposito, che fa i
  marcatori nuovi sul testo di adesso, la metterebbe nel posto sbagliato senza
  dirla vecchia. Si scarta: il server, che ha la versione nuova, ne manderà
  altre. Senza `version` si prende.
- *Le grezze.* L'elenco arrivato si tiene anche com'è, per file, perché una
  code action rimanda al server le segnalazioni con i campi che il deposito
  non conserva (`data`, `source`, `relatedInformation`).
- *La fine.* Alla consegna `exit`, `finding-clear` del produttore: un server
  che non c'è più non ha niente da dire.

Usa C5 per tenerle, C6 per mostrarle, C8 per le colonne, R3 per leggerle. Non
manca niente: è la prima funzione da scrivere, e la prova che i mattoni
bastano.

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
richiesto mentre ce n'è uno in corso prende il suo posto (@richieste). Il
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
  com'è; si ordinano, si rifiuta la modifica se due si sovrappongono, e si
  applicano dall'ultima alla prima, con `delete-region` e `insert`, come fanno i
  commenti e i rettangoli (manuale, `<modifiche-multiple>`); un
  `undo-boundary` prima e dopo fa di ogni buffer un passo di undo. I file
  toccati restano da salvare. Poi lo svuotamento manda al server ciò che ha
  scritto.
- *Il comando.* `workspace/executeCommand` con il comando e i suoi argomenti,
  così come sono arrivati. Il server, di solito, risponde con una richiesta
  `workspace/applyEdit`, che passa dalla modifica appena descritta.

Usa C7, R3, C8, C5 per il contesto, `minibuffer-read`. Al nucleo non manca
niente. Applicare dall'ultima alla prima una lista di sostituzioni, tenendo il
punto al suo posto, è ciò che i commenti fanno già in Rust: una primitiva
agnostica che lo facesse (N7, facoltativa) toglierebbe a questo modulo il
pezzo più delicato e servirebbe ai prossimi.

// ===========================================================================
= Che cosa manca nel nucleo <mancano>
// ===========================================================================

Ogni pezzo è agnostico: il nome e la pagina parlano di buffer, modifiche,
salvataggi e candidati, mai di server o di progetti. Ognuno ha già, o avrà
subito, un cliente che non è `lsp`.

#ref-table(
  columns: (0.9cm, 4.2cm, 1fr, 2.4cm),
  header: ([], [Pezzo], [Che cosa e perché], [Serve a]),
  [N1], [`(buffer-modified-tick &optional BUFFER)`], [La `version` che le porte già incrementano a ogni modifica (manuale, `<due-porte>`), letta. `buffer-modified-p` non basta: resta vero da una modifica all'altra. È il nome di Emacs. Qualunque modulo che ricorda qualcosa calcolato su un testo --- una cache, un indice --- sa da qui se vale ancora.], [prima versione: sincronizzazione intera],
  [N2], [più osservatori: `track-changes-begin` e `track-changes-end` con una `KEY` facoltativa], [Oggi `Buffer::touched` è un `Option<Touched>` e `track-changes-begin` lo rimpiazza, dimenticando ciò che si guardava prima (V1: no, non regge più osservatori). Vim guarda una sessione di Insert; il modulo, nello stesso buffer, gliela toglierebbe, e `.` ripeterebbe la cosa sbagliata. La modifica minima: una piccola lista di `(KEY, Touched)` per buffer, le porte che avvisano ognuno, nil come chiave di chi non ne dà una, così vim non cambia.], [sincronizzazione a tratti],
  [N3], [il testo tolto in `track-changes-end`], [Un quarto elemento, `(START REMOVED INSERTED REMOVED-TEXT)`, nil quando non si sa (il `before` di `Touched` già perso da un undo o da una rilettura). `Touched` tiene già quel testo: è la sua parte fra il prefisso e il suffisso comuni. Serve a chiunque debba ricostruire il testo di prima: un registro delle modifiche, un confronto.], [sincronizzazione a tratti],
  [N4], [`(string-line-column STRING &optional UNIT)`], [Quante righe nuove ci sono in STRING, e la colonna della sua fine nell'unità di C8: la misura che `text/positions.rs` sa fare su un testo, fatta su una stringa. In Lisp costerebbe un giro per carattere, e una regione di un mega tolta un mega di giri.], [sincronizzazione a tratti],
  [N5], [`after-save-hook`], [Gira dopo che un salvataggio ha scritto il file, `C-x C-w` compreso, non dopo un salvataggio automatico. C'è `before-save-hook`, non il suo compagno. Serve a chi lancia qualcosa sul file salvato.], [`didSave`],
  [N6], [`(completion-offer START END CANDIDATES &optional ON-CHOSEN)`], [Offre CANDIDATES per START..END del buffer corrente come `completion-at-point` offre ciò che le fonti hanno dato: filtro, uno solo al posto del testo, più d'uno al presentatore; poi chiama ON-CHOSEN con il candidato messo. Chi chiama controlla prima che il buffer e il punto siano quelli. Serve a ogni fonte che risponde più tardi: una REPL, una ricerca su disco.], [completamento],
)

#ref-table(
  columns: (0.9cm, 4.2cm, 1fr),
  header: ([], [Facoltativo], [Perché non ora]),
  [N7], [applicare una lista di sostituzioni in un passo], [Il modulo lo fa in Lisp, dall'ultima alla prima. Diventa utile quando un secondo cliente lo vorrà.],
  [N8], [un hook all'uscita dell'editor], [Darebbe `shutdown` ai server; `kill_left_over` li ferma comunque, e un server sopporta di essere fermato.],
  [N9], [un'unità in `finding-set`], [Il deposito farebbe i marcatori da colonne `utf-16` quando il file si apre. Con `utf-32` non serve, e con `utf-16` l'errore cade solo dopo i caratteri fuori dal piano di base.],
  [N10], [`after-close-hook` che dice quale buffer], [Il modulo confronta con `buffer-list`.],
)

L'ordine: N1 e N5 prima della prima versione --- server, configurazione,
sincronizzazione intera, segnalazioni, definizione, code action ---; N6 per il
completamento; N2, N3 e N4 insieme per la sincronizzazione a tratti, che
sostituisce l'intera senza cambiare niente altro del modulo.

// ===========================================================================
= Le decisioni <decisioni>
// ===========================================================================

+ *Lo stato del protocollo nel modulo, non nel nucleo.* Le richieste in corso,
  le versioni, le segnalazioni grezze, le capacità sono di LSP; il nucleo ne
  avrebbe bisogno solo se un altro modulo le volesse uguali, e allora non
  sarebbero più di LSP. Il costo è una copia delle segnalazioni, nel deposito e
  nel modulo. Scartato: un campo opaco `:data` nel deposito, che darebbe al
  nucleo un dato che non sa leggere.
+ *Si svuota prima di chiedere, non a tempo.* Un'attesa dopo l'ultimo tasto,
  come fanno altri editor, risparmia messaggi ma lascia un intervallo in cui il
  server non ha il testo su cui gli si chiede; svuotare dopo ogni comando, e
  prima di ogni richiesta, rende quell'intervallo vuoto. Il costo è un
  `didChange` per comando che scrive, che i server reggono.
+ *Il testo intero prima dei tratti.* Un intervallo sbagliato è un difetto
  silenzioso: il server lavora su un testo che non è quello del buffer, e lo
  dice solo con segnalazioni strane. Il testo intero è giusto per
  costruzione, e chiede un solo pezzo di nucleo; i tratti vengono quando il
  nucleo dice anche il testo tolto.
+ *`utf-32` prima di `utf-16`, `utf-8` mai.* Contare come l'editor toglie
  le conversioni e rende giuste le colonne dei file non aperti; `utf-16` è
  l'obbligo del protocollo. Scartato: offrire `utf-8`, che C8 sa convertire,
  ma che sbaglia ogni colonna dopo una lettera accentata dove il testo non c'è.
+ *Le segnalazioni di una versione vecchia si scartano.* Il deposito prende
  le posizioni del produttore come vere per il testo di adesso (D4); una
  versione vecchia lo smentirebbe in silenzio. Il server, che ha la versione
  nuova, ne manda altre.
+ *Un server per radice e configurazione, avviato dal primo buffer che lo
  chiede.* Un progetto con più crate ha un server, non uno per crate, perché
  la radice è quella di `project`, con `.git` che vince; e niente parte in un
  modo per cui l'utente non ha acceso `lsp-mode`. Scartato: un server
  per tutto l'editor con più `workspaceFolders`, che non tutti i server
  reggono.
+ *Niente acceso da sé.* Le voci di `lsp-servers` ci sono, l'accensione è
  dell'utente: un server mancante è un errore a ogni file aperto.
+ *Il completamento è un comando suo, non una fonte.* Una fonte risponde
  dentro il comando, e il server dopo; finché il nucleo non ha N6 non c'è un
  completamento, piuttosto di uno che aspetta. Scartato: una fonte che
  risponde dall'ultima risposta ricordata, che offre al primo tasto i
  candidati della parola di prima.
