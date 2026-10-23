// Il WorkManager e lo scheduler: architettura, logica e uso.
//
// Documento di progetto, implementato. Ciò che vale ancora è nel capitolo
// «Il lavoro in background» del manuale (`architettura-editor.typ`); questo
// file si toglie nella fase D (`documentazione.typ`).

#import "style.typ": *

#show: preamble.with(
  title: "Il WorkManager e lo scheduler",
  subtitle: [Come rsedit esegue il lavoro asincrono: architettura, logica e uso --- documento di progetto],
)

#set text(lang: "it")
#set heading(supplement: [§])
#show table: set par(justify: false)
#set figure(kind: image)
#show raw.where(block: true): it => block(breakable: false, it)
#show table: it => block(breakable: false, it)

// Le figure disegnano il codice in linea piccolo e senza sfondo.
#let canvas = canvas.with(code-size: 7.4pt)

// Un rimando a una scheda del rapporto sui problemi (rapporto-problemi.typ).
// Seguito da un punto e virgola va scritto `#rapporto("X")\;`.
#let rapporto(id) = text(size: 0.85em, fill: warm)[(→ #id)]

// Il riempimento dei nodi che sono thread.
#let thread-fill = accent.lighten(85%)

#outline(depth: 2, indent: auto)

// ===========================================================================
= In breve
// ===========================================================================

rsedit fa molto lavoro lontano dal thread su cui l'utente scrive: colora il
testo, prescansiona le espressioni, controlla i file cambiati, salva le copie
automatiche, esegue i lavori Lisp, i comandi di shell e le ricerche nei file.
Questo documento descrive la struttura che esegue tutto questo lavoro: il
#emph[WorkManager] e il suo #emph[scheduler]. Spiega come sono fatti, quale
logica seguono e come si usano da Rust e da Lisp. I termini sono riassunti
nel glossario in fondo (@glossario).

#caution[
  *Stato: implementato, e assorbito nel manuale.* Le patch del piano (@piano)
  hanno costruito il WorkManager in `work/`, e `background/` non c'è più. Ciò
  che di questo documento vale ancora --- architettura, lavori, logica dello
  scheduler, argomenti, misure, opzioni, thread dedicati, watchdog e prove ---
  è nel capitolo «Il lavoro in background» del manuale
  (`architettura-editor.typ`), descritto sul codice di oggi: il riferimento è
  quello. Qui restano il piano, l'uso da Lisp, che la pagina `background`
  spiega a chi lo scrive, e le idee per dopo; gli esempi mostrano la forma
  prevista prima del codice. Il documento si toglie nella fase D.
]

#key[
  *Le regole, in sei righe.*
  + *Un solo punto d'ingresso.* Tutto il lavoro asincrono passa dal
    WorkManager, ha un nome e un `WorkId`, e si può elencare e fermare.
  + *Due forme di lavoro.* I lavori a passi girano sugli esecutori dello
    scheduler; ciò che può bloccarsi (un processo, il disco) ha un thread
    dedicato, di cui il WorkManager tiene l'handle.
  + *Un passo non aspetta mai.* Fa una quantità limitata di lavoro e dice come
    proseguire.
  + *Lo scheduler misura e segnala, non interrompe.* Un passo troppo lungo
    finisce nel log; la correzione si fa a mano.
  + *Gli eventi non portano dati.* Dicono «vai a guardare»: la fonte della
    verità è lo stato.
  + *Tutto si prova senza thread.* Il nucleo dello scheduler riceve l'ora come
    parametro.
]

== Chi possiede cosa

#figure(
  canvas(height: 6.4cm)[
    #group(0cm, 0.35cm, 16.2cm, 5.9cm, [EditorState])
    #group(0.4cm, 1.0cm, 15.4cm, 5.0cm, [WorkManager --- `Arc`, uno per editor], colour: accent)

    #node(3.1cm, 1.95cm, 4.6cm, 0.8cm, [registro: `WorkId`, nomi, stato, misure], fill: paper)
    #node(7.65cm, 1.95cm, 3.7cm, 0.8cm, [opzioni], fill: paper)
    #node(12.55cm, 1.95cm, 5.7cm, 0.8cm, [argomenti: contatori degli eventi], fill: paper)

    #group(0.8cm, 3.0cm, 8.9cm, 2.65cm, [scheduler])
    #node(3.3cm, 4.35cm, 4.4cm, 1.5cm, [nucleo: \ code, scadenze, sottoscrizioni, regole, misure])
    #node(7.6cm, 4.35cm, 3.4cm, 1.5cm, [esecutori: \ 1 thread, poi N], fill: thread-fill)
    #carrow((5.5cm, 4.35cm), (5.9cm, 4.35cm))

    #node(12.55cm, 3.75cm, 5.7cm, 0.8cm, [thread dedicati: handle + `Stop`], fill: thread-fill)
    #node(12.55cm, 4.95cm, 5.7cm, 0.8cm, [watchdog: osserva, raccoglie, scrive il log], fill: thread-fill)
  ],
  caption: [Il WorkManager e le sue parti. In azzurro i thread: tutto il resto sono dati, raggiunti da qualunque thread attraverso il WorkManager.],
)

// ===========================================================================
= L'architettura
// ===========================================================================

== Chi parla con chi

#figure(
  canvas(height: 7.4cm)[
    #lane(2.0cm, 0cm, 7.3cm, [thread dei comandi], fill: thread-fill, w: 3.2cm)
    #lane(6.6cm, 0cm, 7.3cm, [esecutore], fill: thread-fill, w: 2.6cm)
    #lane(10.9cm, 0cm, 7.3cm, [thread dedicato], fill: thread-fill, w: 2.8cm)
    #lane(14.6cm, 0cm, 7.3cm, [watchdog], fill: thread-fill, w: 2.4cm)

    #msg(2.0cm, 6.6cm, 1.2cm, [`submit(spec, lavoro)`], colour: accent)
    #self-msg(2.0cm, 1.65cm, [tasto → `emit`])
    #msg(2.0cm, 6.6cm, 2.55cm, [sveglia, una per raffica], dash: "dashed")
    #self-msg(6.6cm, 2.95cm, [+20 ms: un passo])
    #msg(6.6cm, 2.0cm, 3.85cm, [callback dovuta (lavoro Lisp)])
    #msg(2.0cm, 10.9cm, 4.55cm, [`spawn("shell: make", corpo)`], colour: accent)
    #msg(2.0cm, 10.9cm, 5.2cm, [`cancel`: `Stop` + azione di sblocco])
    #msg(10.9cm, 14.6cm, 5.85cm, [finito: `join`, panic nel log], dash: "dashed")
    #msg(14.6cm, 6.6cm, 6.5cm, [osserva il passo in corso], dash: "dashed", colour: accent)
  ],
  caption: [I thread e i messaggi tra loro. Le chiamate al WorkManager (`submit`, `spawn`, `cancel`) avvengono sul thread di chi chiama e non aspettano.],
)

- *Il thread dei comandi* chiama il WorkManager: sottomette lavori, avvia
  thread dedicati, ferma, cambia opzioni. Le porte delle modifiche emettono gli
  argomenti, e una raffica di tasti produce una sola sveglia.
- *L'esecutore* esegue i passi uno alla volta. I lavori Lisp non chiamano mai
  il loro modo: lasciano una callback dovuta, che il thread dei comandi esegue
  tra un comando e l'altro, come oggi.
- *I thread dedicati* lavorano per conto loro. Si fermano con `Stop` e con
  un'azione che li sblocca, per esempio uccidere il processo di cui leggono
  l'output.
- *Il watchdog* osserva gli esecutori, raccoglie i thread dedicati finiti
  (`join`) e scrive il file di log. Così una segnalazione arriva nel log anche
  quando ciò che segnala è bloccato.

== Il ciclo di vita del WorkManager

- *Nasce* in `EditorState::new`, prima che `init.lisp` venga valutato: lavori e
  opzioni si possono impostare dalla configurazione.
- *Ha due modalità.* #emph[Con thread] è l'editor vero. #emph[Manuale] è per i
  test: nessun esecutore gira da solo, e il test fa avanzare il lavoro quando
  vuole (@prove).
- *Termina con `shutdown(entro)`*, chiamato dalla TUI all'uscita e, nei test, da
  un oggetto guardia. Smette di accettare lavoro, fa uscire i lavori con
  `Exit::Shutdown`, manda `Stop` ai thread dedicati, aspetta fino alla
  scadenza e abbandona chi resta.

`shutdown` è esplicito per una ragione precisa: i thread tengono una copia di
`EditorState`, che contiene il WorkManager. Il ciclo di riferimenti impedisce
al `Drop` di bastare. È lo stesso ciclo che oggi tiene in vita per sempre lo
scheduler di ogni editor creato, anche nei test.

== I thread dedicati <thread>

Un thread dedicato serve a ciò che può bloccarsi e non si può dividere in
passi: leggere l'output di un processo, una lettura dal disco. Il WorkManager
lo avvia, ne tiene l'handle e lo elenca nel registro come un lavoro.

#figure(
  canvas(height: 3.7cm)[
    #node(2.3cm, 1.0cm, 3.0cm, 0.8cm, [in corso], fill: thread-fill)
    #node(13.2cm, 1.0cm, 3.0cm, 0.8cm, [finito], fill: paper)
    #node(7.7cm, 2.7cm, 3.0cm, 0.8cm, [in chiusura], fill: thread-fill)
    #node(13.2cm, 2.7cm, 3.0cm, 0.8cm, [abbandonato], fill: paper)

    #carrow((3.8cm, 1.0cm), (11.7cm, 1.0cm), label: [il corpo ritorna], dy: -0.25cm)
    #carrow((3.2cm, 1.4cm), (6.2cm, 2.5cm), label: [`cancel`: `Stop` + azione], dx: -1.6cm, dy: 0.35cm)
    #carrow((9.2cm, 2.5cm), (12.2cm, 1.4cm), label: [si ferma], dx: -0.3cm, dy: -0.35cm)
    #carrow((9.2cm, 2.7cm), (11.7cm, 2.7cm), label: [allo shutdown], dy: 0.3cm)
    #note(7.0cm, 3.2cm, colour: warm)[oltre 1 s: segnalato]
  ],
  caption: [Il ciclo di vita di un thread dedicato. Il watchdog fa `join` dei thread finiti e scrive nel log un eventuale panic.],
)

- *`spawn(nome, corpo)`* avvia il thread e restituisce un `WorkId`. Il corpo
  riceve uno `Stop`.
- *Un thread non si uccide.* Rust non ha un modo sicuro per farlo: un thread
  ucciso mentre tiene il lock di un buffer bloccherebbe l'editor al primo
  accesso. `cancel` invece alza il segnale di `Stop`, che il corpo controlla
  nei punti sicuri, ed esegue l'azione registrata con `stop.on_cancel`, che
  sblocca ciò che il thread aspetta: per un comando di shell, uccidere il
  processo chiude le sue pipe.
- *Il `join` non blocca nessuno.* Il watchdog lo fa solo sui thread che
  risultano finiti (`is_finished`). Un panic nel corpo torna dal `join` e va
  nel log.
- *Un thread che non si ferma*, come un `stat` su un disco di rete appeso, resta
  elencato «in chiusura», viene segnalato e, allo `shutdown`, abbandonato.

// ===========================================================================
= I lavori
// ===========================================================================

== Il contratto

```rust
/// Un lavoro a passi. Lo scheduler chiama `step`; il lavoro fa una quantità
/// limitata di lavoro e dice come proseguire.
pub trait Job<B: BufferTrait>: Send + 'static {
    fn step(&mut self, cx: &mut StepContext<'_, B>) -> Result<Step, JobError>;

    /// Chiamata una volta sola, quando il lavoro esce, per qualunque motivo.
    fn exit(&mut self, editor: &EditorState<B>, why: Exit) {}
}

pub enum Step {
    Continue,              // c'è altro da fare: rimettimi in coda
    Sleep(Duration),       // svegliami tra un po'
    Wait,                  // svegliami a un mio evento
    WaitAtMost(Duration),  // a un mio evento, o al più tardi tra un po'
    Done,                  // ho finito
}

pub enum Exit { Finished, Failed, Panicked, Cancelled, Shutdown }

pub struct StepContext<'a, B: BufferTrait> {
    pub editor: &'a EditorState<B>,
    pub woken_by: Wake,    // Start, Turn, Event, Timer
    // …
}

impl<B: BufferTrait> StepContext<'_, B> {
    /// Quanto ha fatto il passo, e su che cosa: la misura deterministica.
    pub fn worked(&mut self, units: u64, on: &str) { /* … */ }
}
```

Un lavoro si sottomette con una `Spec`, che ne dichiara il nome e le
proprietà:

```rust
let id: WorkId = work.submit(
    Spec::new("highlighter")
        .class(Class::High)           // il default è Class::Normal
        .unit("righe")                // l'unità di cx.worked
        .on("buffer-changed"),        // un argomento a cui è sottoscritto
    Box::new(Highlighter),
);
```

== Le regole del passo

- *Non aspetta mai.* Niente I/O che può bloccarsi, che va su un thread
  dedicato; niente `sleep`. Chi deve aspettare lo dice nell'esito.
- *Non tiene lock da un passo all'altro.* Copia sotto lock, lavora senza,
  salva solo se la versione non è cambiata: la regola di oggi.
- *Non guarda l'orologio per decidere quanto fare.* Divide il lavoro in unità
  dei suoi dati (righe, file, passi di carburante) e lo dichiara con
  `cx.worked`. Il tempo lo misura lo scheduler.
- *Non si interrompe.* `cancel` ha effetto alla fine del passo in corso.
- *Gli errori passano dal `Result`.* Lo scheduler li scrive nel log con il nome
  del lavoro e chiama `exit(Failed)`.

== Gli stati

#figure(
  canvas(height: 5.3cm)[
    #node(2.0cm, 1.6cm, 2.6cm, 0.8cm, [pronto], fill: paper)
    #node(7.0cm, 1.6cm, 3.0cm, 1.0cm, [in corso \ (un passo)], fill: thread-fill)
    #group(10.3cm, 0.35cm, 5.9cm, 3.0cm, [in attesa])
    #node(13.25cm, 0.95cm, 5.0cm, 0.62cm, [di una scadenza: `Sleep`], fill: paper)
    #node(13.25cm, 1.8cm, 5.0cm, 0.62cm, [di un evento: `Wait`], fill: paper)
    #node(13.25cm, 2.65cm, 5.0cm, 0.62cm, [dell'uno o dell'altro: `WaitAtMost`], fill: paper)
    #node(7.0cm, 3.95cm, 2.6cm, 0.7cm, [uscito], fill: paper)

    #carrow((3.3cm, 1.42cm), (5.5cm, 1.42cm), label: [scelto], dy: -0.27cm)
    #carrow((5.5cm, 1.8cm), (3.3cm, 1.8cm), label: [`Continue`], dy: 0.27cm)
    #carrow((8.5cm, 1.6cm), (10.3cm, 1.6cm), label: [attende], dy: -0.27cm)
    #carrow((7.0cm, 2.1cm), (7.0cm, 3.6cm), label: [`Done`, errore, panic], dx: 1.25cm)

    #segment((13.25cm, 3.35cm), (13.25cm, 4.75cm), colour: accent, dash: "dashed")
    #segment((13.25cm, 4.75cm), (2.0cm, 4.75cm), colour: accent, dash: "dashed")
    #arrow((2.0cm, 4.75cm), (2.0cm, 2.0cm), colour: accent, dash: "dashed")
    #note(2.3cm, 4.85cm, colour: accent)[la scadenza o l'evento lo riportano «pronto»]
  ],
  caption: [Gli stati di un lavoro. `cancel` e `shutdown` portano a «uscito» da qualunque stato; se il lavoro è in corso, alla fine del passo.],
)

== Le uscite

#ref-table(
  columns: (2.6cm, 1fr),
  header: ([`Exit`], [Quando]),
  [`Finished`], [il passo ha risposto `Done`],
  [`Failed`], [`step` ha restituito un errore],
  [`Panicked`], [il passo è andato in panic: lo scheduler lo intercetta con `catch_unwind`],
  [`Cancelled`], [qualcuno l'ha fermato, o un lavoro con lo stesso nome l'ha sostituito],
  [`Shutdown`], [l'editor si chiude],
)

In `exit` va la pulizia: un contatore da scalare, la callback `on-done` di un
lavoro Lisp. Oggi questo codice è sparso nei rami di `execute`.

== Nomi e sostituzione

Il nome lo dà chi sottomette, e lo tiene il registro. I nomi sono unici:
sottomettere un nome già presente fa uscire il vecchio lavoro con `Cancelled`
e mette in coda il nuovo. È la regola di oggi per i lavori Lisp, estesa a
tutti. Il `WorkId` prende il posto delle generazioni che oggi stanno in
`Runtime`.

// ===========================================================================
= La logica dello scheduler
// ===========================================================================

== Il ciclo dell'esecutore

#figure(
  canvas(height: 6.4cm)[
    #node(7.2cm, 0.6cm, 7.4cm, 0.8cm, [*1* #h(0.3em) dorme fino alla scadenza più vicina o a un messaggio])
    #node(7.2cm, 1.85cm, 7.4cm, 0.8cm, [*2* #h(0.3em) applica i messaggi: `submit`, `cancel`, opzioni, sveglie])
    #node(7.2cm, 3.1cm, 7.4cm, 0.8cm, [*3* #h(0.3em) scadenze passate ed eventi maturi: i lavori diventano pronti])
    #node(7.2cm, 4.35cm, 7.4cm, 0.8cm, [*4* #h(0.3em) sceglie un lavoro pronto (@priorita)])
    #node(7.2cm, 5.6cm, 7.4cm, 0.8cm, [*5* #h(0.3em) esegue un passo: `catch_unwind`, misura, esito], fill: thread-fill)

    #carrow((7.2cm, 1.0cm), (7.2cm, 1.45cm))
    #carrow((7.2cm, 2.25cm), (7.2cm, 2.7cm))
    #carrow((7.2cm, 3.5cm), (7.2cm, 3.95cm))
    #carrow((7.2cm, 4.75cm), (7.2cm, 5.2cm))

    #segment((10.9cm, 5.6cm), (11.6cm, 5.6cm))
    #segment((11.6cm, 5.6cm), (11.6cm, 1.85cm))
    #arrow((11.6cm, 1.85cm), (10.9cm, 1.85cm))
    #note(11.8cm, 3.25cm)[subito, senza dormire: \ tra un passo e l'altro \ rilegge sempre i messaggi]

    #segment((3.5cm, 4.35cm), (2.8cm, 4.35cm))
    #segment((2.8cm, 4.35cm), (2.8cm, 0.6cm))
    #arrow((2.8cm, 0.6cm), (3.5cm, 0.6cm))
    #note(0.2cm, 2.15cm)[niente di pronto: \ torna a dormire]
  ],
  caption: [Il ciclo di un esecutore. Il nucleo decide i punti 2–4 senza leggere l'orologio: l'ora gli arriva come parametro.],
)

== Chi passa prima <priorita>

- *Due classi.* `High` è per ciò che l'utente sta aspettando sullo schermo:
  evidenziatore e prescanner. `Normal` è il default: controllo dei file,
  salvataggio automatico, tutti i lavori Lisp.
- *Una coda per classe, a turno.* Un lavoro entra in fondo alla coda della sua
  classe ogni volta che diventa pronto: sottomesso, `Continue`, scadenza
  passata, evento. Se è l'unico pronto, riparte subito.
- *Prima la classe alta, ma non sempre.* Dopo `high-steps` passi alti di fila
  (4 per default), se un lavoro normale è pronto tocca a lui. La regola si conta
  in passi, non in tempo: è deterministica e si prova senza orologio.

#figure(
  canvas(height: 1.7cm)[
    #for (i, c) in "HHHHNHHHHNHHHHN".clusters().enumerate() {
      node(0.55cm + i * 1.08cm, 0.55cm, 0.86cm, 0.6cm,
        if c == "H" [`E`] else [`I`],
        fill: if c == "H" { thread-fill } else { paper })
    }
    #note(0.2cm, 1.05cm)[`E` = evidenziatore su un file enorme (sempre `Continue`), `I` = indice delle pagine di manuale con `(yield 'continue)`: un passo ogni cinque.]
  ],
  caption: [La regola con `high-steps = 4`. Quando l'evidenziatore ha finito, l'indice ha l'esecutore tutto per sé.],
)

== Le scadenze

- *Un lavoro ha al più una scadenza*, messa da `Sleep` o `WaitAtMost`. Chi ne ha
  più d'una, come il salvataggio automatico (una per buffer), le tiene per sé e
  chiede ogni volta la più vicina.
- *La scadenza si conta dalla fine del passo.* Un lavoro lento non si accoda a
  sé stesso, e un periodo saltato non si recupera.
- *Il ritardo con cui un lavoro parte* rispetto alla scadenza viene misurato e,
  oltre la soglia, segnalato.

== Gli eventi

- *Un argomento è un contatore.* Emetterlo costa un incremento atomico, più una
  sveglia all'esecutore solo se non ne ha già una in arrivo: si può fare anche
  dentro il lock di un buffer.
- *La sottoscrizione è persistente.* Un lavoro ha eventi in sospeso se il
  contatore di un suo argomento ha superato l'ultimo valore che ha visto.
  Quel valore si aggiorna #emph[prima] che il passo parta: un evento che
  arriva durante il passo non va perso.
- *Il ritardo raccoglie le raffiche.* Un lavoro diventa pronto `event-delay`
  dopo il primo evento non ancora visto (20 ms per default), e gli eventi che
  arrivano nel frattempo si sommano a quello.
- *Un `Wait` senza argomenti sottoscritti è un errore.* Il lavoro verrebbe
  dimenticato, quindi lo scheduler lo segnala.

#figure(
  canvas(height: 2.85cm)[
    // Asse del tempo: 0,28 cm per millisecondo, da 0 a 50 ms.
    #let t(ms) = 1.0cm + ms * 0.28cm
    #arrow((t(0), 2.2cm), (t(52), 2.2cm), colour: dim)
    #for ms in (0, 10, 20, 30, 40, 50) {
      segment((t(ms), 2.12cm), (t(ms), 2.28cm), colour: dim)
      note(t(ms) - 0.2cm, 2.35cm)[#ms ms]
    }
    // Le emissioni, con il valore che il contatore assume.
    #for (ms, n) in ((0, 1), (5, 2), (12, 3), (23, 4)) {
      arrow((t(ms), 1.5cm), (t(ms), 2.15cm), colour: warm)
      note(t(ms) + 0.08cm, 1.45cm, colour: warm)[#n]
    }
    // Il ritardo, contato dal primo evento non ancora visto.
    #segment((t(0), 1.2cm), (t(20), 1.2cm), colour: accent, dash: "dashed")
    #segment((t(23), 1.2cm), (t(43), 1.2cm), colour: accent, dash: "dashed")
    #note(t(1), 0.82cm, colour: accent)[`event-delay`]
    // I passi.
    #note(t(20) - 0.1cm, 0.0cm)[visto = 3]
    #node(t(23), 0.62cm, 6 * 0.28cm, 0.42cm, [passo], fill: thread-fill, size: 7pt)
    #note(t(45) - 1.6cm, 0.0cm)[4 > 3: di nuovo]
    #node(t(45), 0.62cm, 4 * 0.28cm, 0.42cm, [passo], fill: thread-fill, size: 7pt)
  ],
  caption: [Tre tasti in 12 ms producono un solo passo. Le frecce sono le emissioni, con il valore del contatore. Il quarto tasto arriva durante il passo: il contatore supera il valore visto, e dopo il ritardo il lavoro riparte.],
)

Gli argomenti iniziali sono due:

#ref-table(
  columns: (3.3cm, 1fr),
  header: ([Argomento], [Chi lo emette]),
  [`buffer-changed`], [`edits::changed` (le porte, undo, redo, `replace-back`), `adopt_text`, `change_major_mode` e la creazione di un buffer],
  [`syntax-changed`], [chi cambia una grammatica o una tabella di sintassi. Il contatore `grammar_epoch` che esiste già è un argomento di questo tipo.],
)

Il modulo dei buffer non conosce lo scheduler: alla creazione, ogni buffer
riceve l'argomento `buffer-changed` già risolto, cioè un contatore con la sua
sveglia.

Gli eventi non portano dati, e non è una dimenticanza. Un messaggio con dati va
consegnato uno per uno, invecchia (la riga indicata vale per una versione già
superata) e, se una coda si riempie, va perso: non può essere la fonte della
verità. Ciò che serve a un lavoro si registra nello stato, dove nasce. Per
esempio, la geometria di una modifica (riga, righe aggiunte o tolte) andrà
nelle cache dell'evidenziatore e del prescanner, per smettere di ricolorare
appena lo stato del lexer torna uguale a prima. I dati che non esistono altrove,
come l'esito di una lettura, viaggeranno in una casella di posta per lavoro,
insieme all'I/O asincrono dal Lisp.

== Più esecutori

Ogni esecutore ha le sue code e il suo conteggio dei passi alti. Un lavoro
viene assegnato all'esecutore meno carico quando entra e ci resta: l'interprete
ha uno stato per thread (carburante, dati di match, `in_worker`), e una fiber
ripresa su un altro thread lo troverebbe diverso. Un sostituto con lo stesso
nome va sullo stesso esecutore del vecchio, così i due non girano mai insieme.
Il numero di esecutori non compare nell'interfaccia.

// ===========================================================================
= Misure, segnalazioni, opzioni
// ===========================================================================

== Che cosa si misura

- *Ogni passo:* lavoro, classe, esecutore, inizio, durata, attesa in coda,
  ritardo sulla scadenza, unità dichiarate, esito. Per i lavori Lisp le unità
  sono i passi di carburante, contati dallo scheduler.
- *Ogni lavoro:* passi, tempo totale, passo più lungo e su che cosa, stato.
- *Ogni esecutore:* occupazione nell'ultimo secondo e passo in corso.
- *Ogni thread dedicato:* avvio, stato, durata.
- *Le callback dovute,* sul thread dei comandi: è lì che una callback lenta
  blocca l'interfaccia.

Il costo per passo è di due letture dell'orologio e qualche somma; una
segnalazione si compone solo quando una regola scatta.

== Le segnalazioni

#ref-table(
  columns: (3.0cm, 5.6cm, 1fr),
  header: ([Segnalazione], [Quando (opzione)], [Cosa riporta]),
  [Passo lungo], [un passo dura più di 40 ms (`long-step`)], [lavoro, durata, unità, costo per unità, su che cosa],
  [Ritardo], [un lavoro pronto aspetta più di 40 ms se alto, 200 ms se normale (`late-high`, `late-normal`)], [di quanto, e chi ha occupato l'esecutore],
  [Carico], [un esecutore occupato per più di metà dell'ultimo secondo (`busy-share`)], [la percentuale e i lavori che la causano],
  [Passo bloccato], [lo stesso passo in corso da più di 1 s, poi a 2 s, 4 s… (`stuck-step`)], [lavoro e tempo; la scrive il watchdog],
  [Panic, errore], [sempre], [lavoro e messaggio; il lavoro esce],
  [Thread che non si ferma], [vivo 1 s dopo `cancel`], [nome e tempo trascorso],
  [Callback lenta], [una callback dovuta oltre 40 ms], [lavoro e durata],
)

La soglia del ritardo è diversa per le due classi perché un lavoro normale,
per progetto, aspetta fino a `high-steps` passi alti. Per ogni coppia lavoro e
regola la prima occorrenza va subito nel log; le successive confluiscono in un
riepilogo, al più uno ogni `summary-every`. Nessuna segnalazione cambia il
comportamento di un lavoro.

#block(breakable: false)[
```text
[BKG] passo lungo: highlighter, 912 ms su app.min.js (3 righe, 304 ms/riga)
[BKG] ritardo: file-watcher partito 230 ms tardi, dietro highlighter (6 passi)
[BKG] carico: 64% dell'ultimo secondo (highlighter 41%, manpage-index 20%)
[BKG] passo bloccato: manpage-index in corso da 2,0 s
[BKG] thread: shell «make» ancora vivo 1,0 s dopo la richiesta di stop
[BKG] riepilogo: highlighter, 12 passi lunghi negli ultimi 10 s (max 912 ms)
```
]

== Le opzioni

#ref-table(
  columns: (3.4cm, 2.2cm, 1fr),
  header: ([Opzione], [Default], [Cosa regola]),
  [`high-steps`], [4], [passi alti di fila prima di uno normale],
  [`event-delay`], [20 ms], [ritardo dopo un evento],
  [`long-step`], [40 ms], [soglia del passo lungo],
  [`late-high`, `late-normal`], [40, 200 ms], [soglie del ritardo],
  [`stuck-step`], [1000 ms], [soglia del passo bloccato],
  [`busy-share`], [50 %], [soglia del carico],
  [`summary-every`], [10 000 ms], [intervallo tra due riepiloghi],
  [`watchdog-every`], [250 ms], [ogni quanto si sveglia il watchdog],
)

Le soglie partono dal turno di 40 ms, la cadenza a cui oggi girano
evidenziatore e renderer. Le opzioni stanno nel WorkManager e si leggono con
operazioni atomiche: una modifica vale dal passo successivo.

// ===========================================================================
= Usarlo
// ===========================================================================

== Da Rust

=== Un lavoro guidato da eventi

```rust
/// Colora, un pezzo alla volta, il buffer che ne ha più bisogno.
struct Highlighter;

impl<B: BufferTrait> Job<B> for Highlighter {
    fn step(&mut self, cx: &mut StepContext<'_, B>) -> Result<Step, JobError> {
        match cx.editor.highlight_one_turn() {
            // Ha colorato delle righe: forse ne restano.
            Some((buffer, lines)) => {
                cx.worked(lines as u64, &buffer);
                Ok(Step::Continue)
            }
            // Nessun buffer indietro: dorme fino al prossimo cambiamento.
            None => Ok(Step::Wait),
        }
    }
}

work.submit(
    Spec::new("highlighter").class(Class::High).unit("righe")
        .on("buffer-changed").on("syntax-changed"),
    Box::new(Highlighter),
);
```

=== Un lavoro periodico

```rust
/// Controlla alcuni file per volta, per sempre.
struct FileWatcher<B: BufferTrait> { next: usize, env: Arc<Env<EditorState<B>>> }

impl<B: BufferTrait> Job<B> for FileWatcher<B> {
    fn step(&mut self, cx: &mut StepContext<'_, B>) -> Result<Step, JobError> {
        if watching(&self.env) {
            let (next, checked) = cx.editor.watch_files_turn(self.next);
            self.next = next;
            cx.worked(checked as u64, "file");
        }
        // Riletto a ogni passo: un (setq watch-file-interval 10) vale subito.
        Ok(Step::Sleep(watch_interval(&self.env)))
    }
}
```

Rileggere l'intervallo a ogni passo corregge anche un difetto di oggi:
`FileWatcher` e `AutoSaver` vengono programmati prima che `init.lisp` sia
valutato, quindi `watch-file-interval` e `auto-save-interval` impostati lì non
hanno mai effetto.

=== Un thread dedicato

```rust
let child = Arc::new(Mutex::new(command.spawn()?));
let stdout = child.lock().unwrap().stdout.take().unwrap();
let killer = child.clone();

let id = work.spawn("shell: make", move |stop: Stop| {
    // Uccidere il processo chiude la pipe: è ciò che sblocca la lettura.
    stop.on_cancel(move || { let _ = killer.lock().unwrap().kill(); });
    for line in BufReader::new(stdout).lines() {
        if stop.is_cancelled() { break; }
        // … accoda la riga al buffer, attraverso una porta
    }
});

work.cancel(id);   // più tardi, da un comando
```

=== Emettere un argomento

```rust
// In edits::changed, sotto il lock del buffer: un incremento atomico.
buf.changed.emit();

// Altrove, un argomento si risolve per nome una volta sola:
let syntax = work.topic("syntax-changed");
syntax.emit();
```

== Da Lisp

=== Worker e `yield`

`define-worker` e `background-call` mantengono la forma di oggi. L'intervallo
facoltativo diventa l'attesa di default, cioè ciò che vale quando la fiber
scrive solo `(yield)`.

```lisp
(yield)                                     ; attesa di default: 40 ms se omessa
(yield 'continue)                           ; Continue: riparti appena puoi
(yield '(sleep 250))                        ; Sleep: tra 250 ms
(yield '(wait buffer-changed))              ; Wait: al prossimo evento indicato
(yield '(wait-at-most 500 buffer-changed))  ; WaitAtMost: evento o 500 ms
(yield 42)                                  ; altro valore: come (yield)
```

Gli argomenti dell'ultima attesa diventano la sottoscrizione del lavoro e
restano validi finché non ne indica altri. L'ultima riga serve alla
compatibilità: chi usa il valore di `yield` per comunicare l'avanzamento non
cambia comportamento.

```lisp
;; Indicizza a pezzi; quando ha finito, dorme fino alla prossima modifica.
(define-worker 'symbol-index
  (fiber
    (while t
      (cond ((index-stale-p)
             (index-one-chunk)
             (yield 'continue))                   ; c'è altro: riparti subito
            (t
             (yield '(wait buffer-changed)))))))  ; niente da fare: aspetta
```

=== `background-call`

```lisp
(background-call 'manpage-index
  (fiber (let ((left manpage-path))
           (while left
             (manpage--index-one (car left))
             (setq left (cdr left))
             (yield 'continue))))     ; un pezzo per volta, senza aspettare 40 ms
  'manpage--index-progress            ; dopo ogni yield, sul thread dei comandi
  'manpage--index-done)               ; alla fine, secondo l'uscita
```

#ref-table(
  columns: (5.0cm, 1fr),
  header: ([Uscita], [`on-done`]),
  [`Finished`], [chiamata con `(NOME t)`],
  [`Failed`, `Panicked`], [chiamata con `(NOME nil)`],
  [`Cancelled`, `Shutdown`], [non chiamata: chi ha fermato il lavoro lo sa già],
)

=== Eventi propri, opzioni e misure

```lisp
;; Un modo avvisa il suo worker senza aspettare una modifica.
(emit-event 'my-mode-refresh)
(define-worker 'my-mode-indexer
  (fiber (while t
           (my-mode-reindex)
           (yield '(wait my-mode-refresh buffer-changed)))))

(set-work-option 'high-steps 6)     ; anche da init.lisp
(work-option 'event-delay)          ; → 20
(work-list)                         ; lavori e thread con stato e misure
;; → ((name "highlighter" kind job class high state waiting
;;     steps 1204 total-ms 412 max-ms 38) …)
```

`stop-worker` ferma un lavoro o un thread per nome; `running-workers` diventa
un filtro di `work-list`.

// ===========================================================================
= Prove e piano
// ===========================================================================

== Come si prova <prove>

#ref-table(
  columns: (3.6cm, 1fr),
  header: ([Livello], [Come]),
  [Il nucleo], [Senza thread, con un orologio finto e lavori finti. È il grosso dei test: un minuto di scheduler simulato richiede pochi microsecondi.],
  [WorkManager, esecutori], [Pochi test con thread veri. Aspettano una condizione con una scadenza, mai con attese fisse; usano opzioni piccole e lavori che si bloccano su un canale controllato dal test.],
  [Editor e Lisp], [Il WorkManager in modalità manuale, con `run_until_idle` e `run_for(durata)`.],
)

Nei test l'editor nasce in modalità manuale: i lavori girano solo quando il
test lo chiede, e i thread veri solo dove un test li vuole. Spariscono così
sia le corse (oggi `colour_fully` colora a mano mentre lo scheduler vero colora
lo stesso editor ogni 40 ms) sia i thread che restano vivi dopo ogni test.

```rust
#[test]
fn a_burst_of_edits_wakes_the_job_once() {
    let work = WorkManager::manual();                   // niente thread
    let probe = ScriptedJob::new([Step::Wait, Step::Wait]);
    let wakes = probe.wakes();                          // registra i risvegli
    work.submit(Spec::new("probe").on("buffer-changed"), Box::new(probe));
    work.run_until_idle(&editor);                       // primo passo: Wait

    let changed = work.topic("buffer-changed");
    for _ in 0..3 {
        changed.emit();
        work.advance(Duration::from_millis(5));         // tre tasti in 15 ms
    }
    work.run_for(&editor, Duration::from_millis(20));   // il ritardo scade
    assert_eq!(*wakes.lock().unwrap(), [Wake::Start, Wake::Event]);
}
```

Gli altri strumenti:

- *Lavori finti:* `ScriptedJob` risponde una sequenza di esiti e registra i
  risvegli; `PanicJob` va in panic; `BlockingJob` resta fermo finché il test
  non lo sblocca.
- *Invarianti su sequenze casuali:* un test genera sottomissioni, `cancel`,
  emissioni ed esiti con un generatore scritto nel test, e controlla dopo ogni
  operazione che ogni lavoro stia in un solo posto, che `exit` arrivi una volta
  sola e che la regola dei `high-steps` valga. Il seme viene stampato, per
  riprodurre un fallimento.
- *Perdite di thread:* dopo `shutdown()` il numero di thread del processo torna
  quello di partenza (su Linux, leggendo `/proc`).

== Il piano <piano>

Il nuovo modulo `work/` nasce accanto a `background/`, e i lavori ci passano uno
alla volta. Ogni passo è una patch che lascia l'editor funzionante e i test
verdi.

#ref-table(
  columns: (0.6cm, 1fr, 2.7cm),
  header: ([], [Contenuto], [Chiude]),
  [0], [Il log non va più in panic e ha un limite (la patch già pronta).], [AVV-3, AVV-4],
  [1], [Il nucleo: `Job`, esiti, code, regola delle classi, scadenze, argomenti, misure e segnalazioni, con l'orologio finto e i test, compresi gli invarianti.], [],
  [2], [Il WorkManager: esecutore, modalità manuale e con thread, registro, opzioni, `shutdown`, test sulle perdite di thread. Nessun lavoro spostato.], [BKG-1],
  [3], [Gli argomenti alla fonte: `buffer-changed`, `syntax-changed`.], [],
  [4], [Evidenziatore e prescanner nel WorkManager, guidati da eventi; i test di colorazione con `run_until_idle`.], [],
  [5], [Controllo dei file e salvataggio automatico, con gli intervalli riletti a ogni passo.], [difetto degli intervalli],
  [6], [I lavori Lisp: `define-worker`, `background-call`, la tabella di `yield`, `emit-event`, opzioni, `work-list`.], [],
  [7], [Comandi di shell e ricerca nei file come thread dedicati, con `cancel`.], [SHL-1, SHL-2, SHL-3],
  [8], [Il watchdog, con la scrittura del log.], [],
  [9], [Via `background/`: `BackgroundScheduler`, `RunNow` e i vecchi trait.], [],
)

Dopo: il resync dell'evidenziatore e del prescanner; la casella di posta e
l'I/O asincrono dal Lisp; l'handle restituito da `spawn` in `risp`; la misura
dei lock lunghi; più esecutori.

// ===========================================================================
= Appendice
// ===========================================================================

== Glossario <glossario>

#ref-table(
  columns: (3.3cm, 1fr),
  header: ([Termine], [Che cos'è]),
  [WorkManager], [La struttura che possiede tutto il lavoro asincrono: registro, opzioni, argomenti, scheduler, thread dedicati, watchdog. Uno per editor, condiviso con `Arc`.],
  [Lavoro], [Qualcosa che lo scheduler esegue a passi. Implementa il trait `Job`.],
  [Passo], [Una chiamata a `Job::step`: una quantità limitata di lavoro.],
  [Esito], [La risposta di un passo: `Continue`, `Sleep`, `Wait`, `WaitAtMost`, `Done`.],
  [Uscita], [La fine di un lavoro, con il suo motivo (`Exit`). Arriva una volta sola.],
  [Classe], [`High` o `Normal`: decide chi passa prima.],
  [Esecutore], [Un thread dello scheduler. All'inizio uno solo; un lavoro resta sempre sullo stesso.],
  [Nucleo], [Lo stato dello scheduler: code, scadenze, sottoscrizioni, regole, misure. Non ha thread né orologio.],
  [Argomento], [Un evento con un nome: un contatore che cresce a ogni emissione.],
  [Thread dedicato], [Un thread per un lavoro che può bloccarsi, con handle e segnale di stop (`Stop`).],
  [`WorkId`], [Il numero di un lavoro o di un thread dedicato. Non si riusa mai.],
  [Opzione], [Un valore che regola lo scheduler, modificabile da Lisp mentre l'editor gira.],
  [Segnalazione], [Una riga `[BKG]` nel log, quando una misura supera la sua soglia.],
)

== Le decisioni

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Tema], [Decisione]),
  [Struttura], [Il WorkManager possiede thread dedicati, registro e opzioni, e contiene lo scheduler, che esegue solo lavori a passi e non crea thread altrui.],
  [Lavoro], [Il trait `Job`. Il nome lo dà il registro; nomi unici, il nuovo sostituisce il vecchio; il `WorkId` non si riusa.],
  [Esiti], [`Continue`, `Sleep`, `Wait`, `WaitAtMost`, `Done`; errori nel `Result`; `exit` una volta sola, con il motivo.],
  [Partenza], [Subito, salvo un ritardo esplicito.],
  [`yield`], [Generico, con un'attesa di default quando la fiber non chiede niente.],
  [`on-done`], [Secondo il motivo dell'uscita.],
  [Priorità], [Due classi, a turno; un passo normale ogni `high-steps` alti, opzione modificabile a runtime; i lavori Lisp sono normali.],
  [Scadenze], [Una per lavoro, contata dalla fine del passo.],
  [Eventi], [Argomenti con nome, senza dati, letti tramite contatori; sottoscrizione persistente; ritardo dal primo evento non visto; argomenti iniziali `buffer-changed` e `syntax-changed`.],
  [Dati], [Non negli argomenti. La geometria delle modifiche va nelle cache (resync); i messaggi con dati in una casella per lavoro, con l'I/O dal Lisp.],
  [Misure], [Per passo, lavoro, esecutore, thread dedicato e callback; soglie come opzioni; prima occorrenza subito, poi riepiloghi; `work-list`.],
  [Opzioni], [Primitive `work-option`, `set-work-option`, `work-options`.],
  [Prove], [Tre livelli; modalità manuale nei test; invarianti su sequenze casuali; test sulle perdite di thread.],
  [Restano valide], [Gli sforamenti si segnalano e basta; niente budget di tempo nei lavori; il turno di 40 ms come riferimento; yield esplicito nel Lisp; segnalazioni nel log; sola libreria standard; `spawn` resta nell'interprete.],
)
