#import "style.typ": *

#show: preamble.with(
  title: "L'interprete Lisp di rsedit",
  subtitle: [Come funziona, dal testo al valore --- manuale per lo sviluppatore],
)

#set text(lang: "it")

// Le figure di questo manuale disegnano il codice in linea piccolo, così una
// casella può nominare un tipo e restare leggibile.
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

Questo manuale spiega come funziona l'interprete Lisp che vive in
`core/src/lisp/`. Segue un'espressione dal testo sorgente fino al valore e, a
ogni tappa, descrive le strutture che attraversa, l'algoritmo che le
trasforma e il motivo per cui l'algoritmo è fatto così. È scritto per chi deve
modificare l'interprete, estenderlo o incorporarlo in un altro programma.

L'interprete è #emph[agnostico rispetto al contesto]. Non conosce buffer,
finestre, tasti o file: ogni suo tipo è generico su un parametro
`T: LispContext`, e l'editor è soltanto uno dei `T` possibili. Il manuale
compagno, #emph[L'editor di rsedit] (`architettura-editor.typ`), descrive
l'altro lato del confine. Il documento `lisp-fibers.typ` approfondisce la
storia e le scelte di progetto delle fiber; qui se ne descrive il meccanismo.

#key[
  Il confine è verificato, non solo dichiarato: il test `tests::layering_tests`
  fallisce se un qualsiasi file sotto `lisp/` nomina un tipo dell'editor. Tutto
  ciò che questo manuale dice vale quindi per qualunque host incorpori
  l'interprete, compreso il contesto vuoto `()` usato dai test.
]

== Come leggere questo manuale

Il @panoramica dà il quadro d'insieme: i file, le quattro strutture portanti e
il viaggio di un'espressione, con due esempi seguiti passo per passo. I capitoli
dal @modello-dati al @thread descrivono ciascun meccanismo nell'ordine in cui
un'espressione lo incontra: i dati, il lettore, gli ambienti, il valutatore, gli
errori, le fiber, il carburante, i thread. Gli ultimi capitoli sono di
riferimento: il contratto con l'host, le primitive di base, le differenze da
Emacs Lisp e le ricette per estendere l'interprete.

Gli estratti di codice sono presi dal sorgente e semplificati: commenti tolti,
parametri generici abbreviati, rami di errore ripetitivi riassunti. Ogni riga di
un estratto corrisponde a una riga del file indicato sopra di esso.

== Comportamento e difetti

Questo manuale descrive il comportamento #emph[attuale] del codice, anche dove
quel comportamento è un difetto. I difetti sono descritti, con la loro
riproduzione e una correzione proposta, in un documento separato, il
#emph[Rapporto sui problemi del codice] (`rapporto-problemi.typ`). Dove il
manuale descrive un punto che il rapporto considera un difetto, un rimando
come #rapporto("INT-1") indica la scheda corrispondente. Quando il difetto
viene corretto, si aggiorna la descrizione qui e si toglie la scheda là.

== Convenzioni

- I nomi in `monospazio` sono identificatori del codice: tipi e funzioni Rust
  (`eval_step`), primitive e forme Lisp (`condition-case`), file (`eval.rs`).
  I percorsi sono relativi a `core/src/lisp/`.
- «Forma» indica sempre un pezzo di #emph[codice]; «valore» o «dato» un
  risultato. La distinzione ha una controparte precisa nei tipi
  (@sintassi-dati).
- Nei diagrammi le caselle grigie sono dati, quelle bianche sono funzioni o
  passi, quelle azzurre appartengono all'host, quelle rosate sono errori.

// ===========================================================================
= Panoramica <panoramica>
// ===========================================================================

== I file e i livelli

#ref-table(
  columns: (auto, 1fr),
  header: ([File], [Contenuto]),
  [`mod.rs`], [Dichiarazione dei moduli e superficie esportata verso l'editor.],
  [`context.rs`], [Il trait `LispContext`: il contratto con l'host (@contratto).],
  [`lispexp.rs`], [`LispExp`, il tipo di ogni valore e di ogni forma; l'obarray dei simboli; uguaglianza e stampa.],
  [`types.rs`], [`ConsCell`, `Lambda`, `SharedAtom`, `Frame`, `FiberState`, `SharedFiber`, `LispPrimitive`.],
  [`parser.rs`], [Il lettore: lexer a stati e parser a discesa ricorsiva (@lettore); `form_to_data`.],
  [`environment.rs`], [`Env`: variabili, funzioni, macro e proprietà dei simboli (@ambienti).],
  [`eval.rs`], [Il valutatore: trampolino, forme speciali, chiamate, sospensioni (@valutatore).],
  [`error.rs`], [`EvalError`: errori e uscite non locali (@errori).],
  [`utils.rs`], [Arità, grammatica dei parametri, legame degli argomenti, conversione dati → forme, mappatura errori → condizioni.],
  [`fuel.rs`], [Il meccanismo del carburante (@carburante). Non dipende da nulla e il valutatore non lo usa direttamente: serve all'host per implementare `consume_fuel`.],
  [`handshake.rs`], [`bootstrap_vm`: crea l'ambiente radice, installa le primitive e verifica che l'interprete funzioni.],
  [`base/`], [Le 86 primitive indipendenti dall'host, in dieci moduli (@primitive-base).],
  [`tests/`], [I test dell'interprete, eseguiti con il contesto `()`.],
)

I file formano livelli: ognuno usa quelli sotto di sé e mai quelli sopra. La
figura è ricavata dalle righe `use` dei sorgenti; una freccia significa «usa».
Le dipendenze che scavalcano un livello non sono disegnate: `handshake.rs`, per
esempio, chiama direttamente anche `eval` e `Parser`.

#canvas(height: 6.3cm)[
  #node(6.6cm, 0.4cm, 13.0cm, 0.7cm, [`handshake.rs` --- `bootstrap_vm`: la radice, le primitive, lo script di verifica])
  #node(6.6cm, 1.6cm, 13.0cm, 0.7cm, [`base/` --- le 86 primitive di base, in dieci moduli], fill: paper)
  #node(4.9cm, 2.8cm, 9.6cm, 0.7cm, [`eval.rs` --- il valutatore])
  #node(11.6cm, 2.8cm, 3.0cm, 0.7cm, [`parser.rs`])
  #node(1.65cm, 4.0cm, 3.1cm, 0.7cm, [`environment.rs`])
  #node(5.0cm, 4.0cm, 2.4cm, 0.7cm, [`utils.rs`])
  #carrow((3.8cm, 4.0cm), (3.2cm, 4.0cm))
  #node(8.05cm, 4.0cm, 2.9cm, 0.7cm, [`error.rs`])
  #node(6.6cm, 5.3cm, 13.0cm, 0.75cm, [`lispexp.rs` · `types.rs` · `context.rs` --- i tipi, che si nominano a vicenda], fill: paper)
  #carrow((6.6cm, 0.75cm), (6.6cm, 1.25cm))
  #carrow((4.9cm, 1.95cm), (4.9cm, 2.45cm))
  #carrow((11.6cm, 1.95cm), (11.6cm, 2.45cm))
  #carrow((2.6cm, 3.15cm), (1.65cm, 3.65cm))
  #carrow((5.0cm, 3.15cm), (5.0cm, 3.65cm))
  #carrow((7.2cm, 3.15cm), (8.05cm, 3.65cm))
  #carrow((1.65cm, 4.35cm), (1.65cm, 4.92cm))
  #carrow((5.0cm, 4.35cm), (5.0cm, 4.92cm))
  #carrow((8.05cm, 4.35cm), (8.05cm, 4.92cm))
  #carrow((11.6cm, 3.15cm), (11.6cm, 4.92cm))
  #node(14.9cm, 2.8cm, 2.4cm, 0.7cm, [`fuel.rs`], stroke-colour: dim)
  #note(13.75cm, 3.3cm)[nessuna dipendenza;\ lo usa l'host]
]

== Le quattro strutture portanti

Quasi tutto l'interprete è il gioco di quattro tipi. Conviene conoscerli prima
di leggere qualunque altra cosa.

#ref-table(
  columns: (2.6cm, 1fr, 1fr),
  header: ([Tipo], [Che cos'è], [Chi lo crea e chi lo consuma]),
  [`LispExp<T>`], [Ogni valore e ogni pezzo di codice: numeri, simboli, stringhe, liste, chiusure, primitive, atomi, fiber (@modello-dati).], [Lo crea il lettore (dal testo) o una primitiva (a tempo di esecuzione); lo consuma il valutatore.],
  [`Env<T>`], [Una catena di tabelle che associa nomi a valori, in tre spazi dei nomi separati (@ambienti).], [Lo crea `bootstrap_vm` (la radice) e ogni chiamata, `let`, ciclo o fiber (un anello figlio); lo consultano il valutatore e le primitive.],
  [`EvalError<T>`], [Il ramo `Err` di ogni valutazione: un errore, un `throw` o una sospensione (@errori).], [Lo producono le forme e le primitive; lo fermano `catch`, `condition-case`, `resume`, oppure arriva all'host.],
  [`T: LispContext`], [L'host: chi paga il carburante, tiene la pila delle chiamate e riceve la diagnostica (@contratto).], [Lo fornisce l'host; il valutatore lo riceve per riferimento a ogni passo.],
)

== Il percorso di un'espressione

Un programma attraversa sempre le stesse tappe. La figura le mostra con il
tipo che passa dall'una all'altra.

#canvas(height: 6.7cm)[
  #node(3.4cm, 0.4cm, 4.2cm, 0.7cm, [testo sorgente (`&str`)], fill: paper)
  #carrow((3.4cm, 0.75cm), (3.4cm, 1.2cm))
  #node(3.4cm, 1.6cm, 4.2cm, 0.8cm, [`Parser`: lexer + parser])
  #arrow((3.4cm, 2.0cm), (3.4cm, 2.45cm), label: [`next()`], label-dx: 5pt, label-dy: -5pt)
  #node(3.4cm, 2.85cm, 4.2cm, 0.8cm, [`LispExp::Form` (sintassi)], fill: paper)
  #arrow((3.4cm, 3.25cm), (3.4cm, 3.75cm), label: [`eval`], label-dx: 5pt, label-dy: -5pt)
  #node(3.4cm, 4.2cm, 4.2cm, 0.9cm, [*valutatore*\ trampolino di `eval_step`])
  #carrow((2.4cm, 4.65cm), (2.0cm, 5.4cm))
  #carrow((4.4cm, 4.65cm), (4.8cm, 5.4cm))
  #node(2.0cm, 5.9cm, 2.6cm, 0.9cm, [`Ok(LispExp)`\ un valore], fill: paper)
  #node(4.8cm, 5.9cm, 2.6cm, 0.9cm, [`Err(EvalError)`\ errore o salto], fill: warm.lighten(85%))

  #node(12.2cm, 0.5cm, 6.2cm, 0.8cm, [`base/`: le primitive di base], fill: paper)
  #carrow((12.2cm, 0.9cm), (12.2cm, 1.95cm), label: [installate da `bootstrap_vm`], dx: 2.0cm)
  #node(12.2cm, 2.45cm, 6.2cm, 1.0cm, [`Env`: catena di tabelle\ variabili · funzioni · macro · proprietà])
  #carrow((12.2cm, 3.85cm), (12.2cm, 2.95cm), label: [registra le sue primitive], dx: 1.9cm, colour: accent)
  #node(12.2cm, 4.35cm, 6.2cm, 1.0cm, [`T: LispContext` (l'host)\ carburante · pila delle chiamate · diagnostica], fill: accent.lighten(85%))
  #carrow((5.5cm, 3.95cm), (9.1cm, 2.6cm), label: [cerca e lega i nomi], dy: -8pt)
  #carrow((5.5cm, 4.45cm), (9.1cm, 4.45cm), label: [`consume_fuel(1)` a ogni passo], dy: -7pt, colour: accent)
]

- Il #emph[lettore] (`Parser`) produce un albero di `LispExp`. Le liste del
  codice sono `LispExp::Form`, cioè vettori, non catene di cons
  (@sintassi-dati).
- Il #emph[valutatore] (`eval`) riduce l'albero in un ciclo, il trampolino, che
  esegue le chiamate in coda senza consumare lo stack di Rust (@trampolino).
- L'#emph[ambiente] (`Env`) è una catena di tabelle: ogni chiamata di funzione
  e ogni `let` ne aggiunge un anello (@ambienti).
- Il #emph[contesto] è l'host. Il valutatore lo chiama a ogni passo per far
  pagare il carburante, e in pochi altri punti per la pila delle chiamate, le
  pulizie e i nuovi thread (@contratto). Le primitive dell'host non passano dal
  contesto: l'host le registra nella stessa radice delle primitive di base, e
  il valutatore le trova per nome come tutte le altre.
- Il risultato è un `Result`. Il ramo `Err` porta sia gli errori sia le uscite
  non locali --- `throw`, `signal` e `yield` --- perché in Rust srotolare lo
  stack si fa propagando un `Err` con `?` (@errori).

Il punto d'ingresso per un host è quindi sempre lo stesso:

```rust
let env = bootstrap_vm(&ctx)?;                 // ambiente radice + primitive
let ast = Parser::new("(+ 1 2)").next()?;      // testo -> LispExp
let value = eval(&ast, env.clone(), &ctx)?;    // LispExp -> valore
```

== Primo esempio: una chiamata a una primitiva

Il diagramma di sequenza segue la valutazione di `(+ 1 (* 2 3))`. Il tempo
scorre verso il basso; le frecce continue sono chiamate, quelle tratteggiate
risposte.

#canvas(height: 9.7cm)[
  #lane(1.5cm, 0.0cm, 9.6cm, [`eval`])
  #lane(5.2cm, 0.0cm, 9.6cm, [`eval_step_permitted`], w: 3.4cm)
  #lane(8.9cm, 0.0cm, 9.6cm, [`Env`])
  #lane(11.7cm, 0.0cm, 9.6cm, [`ctx` (l'host)], fill: accent.lighten(85%))
  #lane(14.6cm, 0.0cm, 9.6cm, [primitiva `+`])

  #msg(1.5cm, 5.2cm, 1.2cm, [`(+ 1 (* 2 3))`])
  #msg(5.2cm, 11.7cm, 1.7cm, [`consume_fuel(1)`], colour: accent)
  #msg(5.2cm, 8.9cm, 2.2cm, [`get_macro("+")`])
  #msg(8.9cm, 5.2cm, 2.65cm, [`None`], dash: "dashed")
  #seq-note(5.4cm, 2.85cm)[`+` non è una forma speciale né una macro:\ si valutano gli argomenti, da sinistra]
  #msg(5.2cm, 1.5cm, 3.8cm, [`eval(1)`])
  #msg(1.5cm, 5.2cm, 4.25cm, [`Ok(1)`], dash: "dashed")
  #msg(5.2cm, 1.5cm, 4.8cm, [`eval((* 2 3))`])
  #seq-note(0.15cm, 5.0cm)[stesso percorso,\ un livello più giù]
  #msg(1.5cm, 5.2cm, 5.85cm, [`Ok(6)`], dash: "dashed")
  #msg(5.2cm, 8.9cm, 6.35cm, [`get_function("+")`])
  #msg(8.9cm, 5.2cm, 6.8cm, [`Primitive`], dash: "dashed")
  #msg(5.2cm, 11.7cm, 7.3cm, [`push_call_frame("+")`], colour: accent)
  #msg(5.2cm, 14.6cm, 7.8cm, [`pointer(&[1, 6], env, ctx)`])
  #msg(14.6cm, 5.2cm, 8.25cm, [`Ok(7)`], dash: "dashed")
  #msg(5.2cm, 11.7cm, 8.75cm, [`pop_call_frame()`], colour: accent)
  #msg(5.2cm, 1.5cm, 9.25cm, [`Done(7)`], dash: "dashed")
]

Il ciclo di `eval` riceve `Done(7)` e restituisce `Ok(7)`. Ogni passo di
`eval_step_permitted` costa un'unità di carburante, e qui i passi sono cinque:
la forma esterna, `1`, la forma interna, `2` e `3`. Le teste `+` e `*` non si
valutano: sono nomi cercati nello spazio delle funzioni. Un simbolo in testa a
una forma non è mai una variabile.

== Secondo esempio: una funzione ricorsiva in coda

La funzione seguente calcola il fattoriale con un accumulatore. La chiamata
ricorsiva è l'ultima cosa che il ramo «altrimenti» dell'`if` fa, cioè è in
#emph[posizione di coda].

```lisp
(defun fatt (n acc)
  (if (= n 0) acc (fatt (- n 1) (* n acc))))
(fatt 3 1)   ; => 6
```

La tabella segue il ciclo di `eval` (il trampolino, @trampolino) durante la
valutazione di `(fatt 3 1)`. Ogni riga è un giro del ciclo: la forma corrente,
l'ambiente in cui va valutata e ciò che il passo restituisce.

#ref-table(
  columns: (0.9cm, 4.1cm, 1.6cm, 1fr),
  header: ([N.], [Forma corrente], [Amb.], [Esito del passo]),
  [1], [`(fatt 3 1)`], [radice], [argomenti valutati (3, 1); nuovo ambiente E1 figlio della radice con `n=3`, `acc=1`; il corpo ha una sola forma, quindi `TailCall((if …), E1)`],
  [2], [`(if (= n 0) acc (fatt …))`], [E1], [la condizione è falsa: `TailCall((fatt (- n 1) (* n acc)), E1)`],
  [3], [`(fatt (- n 1) (* n acc))`], [E1], [argomenti 2 e 3; E2 figlio della #emph[radice] con `n=2`, `acc=3`; `TailCall((if …), E2)`],
  [4], [`(if …)`], [E2], [`TailCall((fatt …), E2)`],
  [5], [`(fatt …)`], [E2], [E3 con `n=1`, `acc=6`; `TailCall((if …), E3)`],
  [6, 7], [`(if …)`, `(fatt …)`], [E3], [E4 con `n=0`, `acc=6`; `TailCall((if …), E4)`],
  [8], [`(if (= n 0) acc …)`], [E4], [la condizione è vera: `TailCall(acc, E4)`],
  [9], [`acc`], [E4], [`Done(6)`: il ciclo restituisce `Ok(6)`],
)

Il ciclo non cresce: ogni chiamata in coda #emph[sostituisce] la coppia
(forma, ambiente) corrente invece di aggiungere un frame allo stack di Rust.
Gli ambienti E1…E4 non si annidano l'uno nell'altro: ognuno è figlio
dell'ambiente in cui `fatt` è stata #emph[definita], la radice (@scope). Gli
argomenti, invece, sono valutati con chiamate ricorsive a `eval`, perché il
loro valore serve prima della chiamata: è la differenza tra una ricorsione in
coda, che non consuma stack, e una che non lo è (@trampolino).

== Gli invarianti su cui si regge tutto

Sei regole attraversano tutto il codice. Ogni scelta descritta nei capitoli
successivi ne è una conseguenza.

+ *Il codice è `Form`, i dati sono `Cons`.* Il lettore produce vettori,
  perché il valutatore accede alle liste di codice per posizione; i valori sono
  catene di celle, perché `car` e `cdr` devono costare O(1) (@sintassi-dati).
+ *Clonare non copia.* Ogni contenitore sta dietro un `Arc`: il valutatore
  clona forme, valori e ambienti a ogni passo, e ogni clone costa un incremento
  di un contatore.
+ *Un passo, un'unità di carburante.* Il valutatore paga all'ingresso di ogni
  passo, prima di sapere che cosa farà (@carburante).
+ *Errori, salti e sospensioni sono tutti `Err`.* Lo srotolamento dello stack
  è affidato a `?`, e nessuna funzione intermedia deve sapere che cosa la sta
  attraversando (@errori).
+ *Nessun lock resta preso mentre gira codice Lisp.* La ricerca negli ambienti
  prende un lock alla volta; `resume` toglie la pila della fiber e rilascia il
  lock prima di eseguirla.
+ *L'interprete non conosce l'host.* Tutto ciò che dipende dall'host passa
  dai metodi di `LispContext` o dalle primitive che l'host registra.

// ===========================================================================
= Il modello dei dati <modello-dati>
// ===========================================================================

== `LispExp`, il tipo di tutto

Ogni valore e ogni pezzo di codice è un `LispExp<T>`. Il parametro `T` è il
contesto: compare qui perché alcune varianti (lambda, primitive, fiber)
racchiudono codice che, eseguito, avrà bisogno dell'host.

#estratto("lispexp.rs")[
```rust
pub enum LispExp<T: LispContext> {
    Form(Arc<Vec<LispExp<T>>>),          // sintassi: ciò che produce il lettore
    Cons(Arc<ConsCell<T>>),              // dati: una cella di lista
    Vector(Arc<Vec<LispExp<T>>>),
    Map(Arc<HashMap<String, LispExp<T>>>),
    Number(f64),
    Symbol(Arc<String>),                 // internato nell'obarray
    String(Arc<String>),
    Lambda(Arc<Lambda<T>>),
    Primitive { pointer: LispPrimitive<T>, doc: Arc<Cow<'static, str>> },
    Atom(SharedAtom<T>),                 // Arc<RwLock<LispExp<T>>>
    Fiber(SharedFiber<T>),               // Arc<RwLock<FiberState<T>>>
}
```
]

#ref-table(
  columns: (auto, auto, 1fr),
  header: ([Variante], [Contenuto], [Note]),
  [`Form`], [`Arc<Vec<LispExp>>`], [Un nodo di #emph[sintassi]: ciò che il lettore produce e `eval` smista. Mai un valore a tempo di esecuzione.],
  [`Cons`], [`Arc<ConsCell>`], [Una lista di #emph[dati]. Termina in `nil` (lista propria) o in altro (lista puntata).],
  [`Vector`], [`Arc<Vec<LispExp>>`], [`[a b c]`. Valutare un vettore ne valuta gli elementi.],
  [`Map`], [`Arc<HashMap<String, LispExp>>`], [`{chiave valore ...}`. Chiavi simbolo o stringa, conservate come testo; valutarla ne valuta i valori. L'ordine non è conservato.],
  [`Number`], [`f64`], [L'unico tipo numerico. Si stampa come intero quando lo è (`3`, non `3.0`).],
  [`Symbol`], [`Arc<String>`, internato], [Due simboli con lo stesso nome condividono lo stesso `Arc`.],
  [`String`], [`Arc<String>`], [Immutabile; clonarla costa un incremento del contatore.],
  [`Lambda`], [`Arc<Lambda>`], [Una chiusura: parametri, corpo e ambiente catturato. Sono `Lambda` anche le funzioni di `defun` e le macro.],
  [`Primitive`], [puntatore a `fn` + doc], [Una funzione scritta in Rust.],
  [`Atom`], [`SharedAtom`], [L'unico contenitore mutabile; attraversa i thread.],
  [`Fiber`], [`SharedFiber`], [Un programma sospendibile (@fiber).],
)

== Clonare non copia

Tutti i contenitori stanno dietro un `Arc`: clonare un `LispExp` non copia mai
i dati, incrementa un contatore. Per questo il valutatore può clonare forme e
valori a ogni passo senza preoccuparsi del costo. La figura mostra due
variabili che condividono la stessa lista e una forma che nomina una delle due.

#canvas(height: 3.6cm)[
  #node(1.2cm, 0.5cm, 1.6cm, 0.6cm, [`a`], fill: paper)
  #node(1.2cm, 1.5cm, 1.6cm, 0.6cm, [`b`], fill: paper)
  #node(4.6cm, 1.0cm, 1.5cm, 0.6cm, [1 | •])
  #node(6.9cm, 1.0cm, 1.5cm, 0.6cm, [2 | •])
  #node(9.2cm, 1.0cm, 1.5cm, 0.6cm, [3 | `nil`])
  #carrow((2.0cm, 0.5cm), (3.85cm, 0.9cm))
  #carrow((2.0cm, 1.5cm), (3.85cm, 1.1cm))
  #carrow((5.2cm, 1.0cm), (6.15cm, 1.0cm))
  #carrow((7.5cm, 1.0cm), (8.45cm, 1.0cm))
  #node(12.9cm, 1.0cm, 3.8cm, 0.6cm, [`Form[car, b]`], fill: paper)
  #note(10.4cm, 1.75cm)[la forma contiene il simbolo `b`, non la lista:\ è codice, e `b` sarà cercato quando la forma è valutata]
  #note(0.2cm, 2.5cm)[`(setq a '(1 2 3))` · `(setq b a)`: un solo `Arc<ConsCell>` per cella, condiviso. Nessuna copia.\ `(eq a b)` è vero perché è la stessa cella; `(equal a b)` lo sarebbe anche per due copie distinte.]
]

La condivisione è sicura perché le liste sono #emph[immutabili]: non esistono
`setcar` né `setcdr`, e nessuna primitiva modifica una cella esistente. Le
primitive che «cambiano» una lista (`append`, `reverse`, `add-to-list`) ne
costruiscono una nuova. L'unico contenitore che si può modificare sul posto è
l'atomo, che per questo ha un lock.

== Costruire e scorrere le liste

Tre costruttori creano liste di dati, e tutti piegano un vettore #emph[da
destra], perché una lista si costruisce dalla coda verso la testa:

#estratto("lispexp.rs")[
```rust
pub fn proper_list(items: Vec<LispExp<T>>) -> LispExp<T> {
    items.into_iter().rev()
        .fold(LispExp::nil(), |cdr, car| LispExp::cons(car, cdr))
}
pub fn improper_list(items: Vec<LispExp<T>>, tail: LispExp<T>) -> LispExp<T> {
    items.into_iter().rev().fold(tail, |cdr, car| LispExp::cons(car, cdr))
}
```
]

#canvas(height: 2.6cm)[
  #node(1.6cm, 0.5cm, 2.6cm, 0.6cm, [`vec![1, 2, 3]`], fill: paper)
  #note(3.2cm, 0.25cm)[`rev()` → 3, 2, 1]
  #node(1.6cm, 1.9cm, 1.6cm, 0.6cm, [`nil`])
  #carrow((2.45cm, 1.9cm), (3.55cm, 1.9cm), label: [3], dy: -7pt)
  #node(4.6cm, 1.9cm, 1.9cm, 0.6cm, [3 | `nil`])
  #carrow((5.6cm, 1.9cm), (6.7cm, 1.9cm), label: [2], dy: -7pt)
  #node(8.1cm, 1.9cm, 2.6cm, 0.6cm, [2 | (3)])
  #carrow((9.45cm, 1.9cm), (10.55cm, 1.9cm), label: [1], dy: -7pt)
  #node(12.3cm, 1.9cm, 3.2cm, 0.6cm, [1 | (2 3)])
  #note(7.0cm, 0.25cm)[ogni passo della piega: `cons(elemento, accumulato)`]
]

Per scorrere una lista ci sono due modi. `iter()` restituisce un `ConsIter`
che produce i `car` uno alla volta e si ferma al primo `cdr` che non è una
cella: una lista puntata viene vista come il suo prefisso proprio. `split_list()`
raccoglie invece `(elementi, coda finale)`, e la coda dice se la lista era
propria (`nil`) o puntata. Chi deve distinguere i due casi, come `data_to_form`
o la stampa, usa `split_list`.

== `nil`, `t` e la verità

`nil` e `t` sono simboli. Il loro `Arc` è conservato in una `OnceLock`, per cui
`LispExp::nil()` e `LispExp::t()` non toccano mai un lock: il valutatore
costruisce `nil` all'uscita di quasi ogni forma, e non deve pagare l'obarray.

#estratto("lispexp.rs")[
```rust
pub fn is_nil(&self) -> bool {
    match self {
        LispExp::Symbol(s) => s.as_str() == "nil",
        LispExp::Form(l) => l.is_empty(),     // la forma vuota ()
        _ => false,
    }
}
pub fn is_truthy(&self) -> bool { !self.is_nil() }
```
]

È falso soltanto `nil`, insieme alla forma vuota `()`; tutto il resto, compresi
`0` e la stringa vuota, è vero. I simboli che iniziano con `:` (le
#emph[keyword]) valgono sé stessi, come `nil` e `t`, senza bisogno di essere
legati: il valutatore li riconosce prima di cercare la variabile (@smistamento).

== L'obarray

L'obarray è la tabella dei nomi dei simboli: `intern(nome)` restituisce
l'#emph[unico] `Arc<String>` per quel nome, così due simboli con lo stesso nome
sono lo stesso puntatore e `eq` sui simboli è un confronto di puntatori.

#estratto("lispexp.rs")[
```rust
pub fn intern(name: String) -> Arc<String> {
    // il caso comune: un lock di lettura e il nome c'è già
    if let Some(found) = obarray().read().unwrap().get(&name) {
        return found.clone();
    }
    // solo la prima volta che un nome compare: il lock di scrittura
    obarray().write().unwrap()
        .entry(name.clone())
        .or_insert_with(|| Arc::new(name))
        .clone()
}
```
]

La tabella è una `static` del processo, condivisa da tutti gli interpreti e da
tutti i thread. Può esserlo perché contiene soltanto nomi e mai un `LispExp`:
Rust non ammette `static` generiche, e una tabella che contenesse valori
dovrebbe essere generica su `T`. Per lo stesso motivo le #emph[proprietà] dei
simboli non stanno nel simbolo ma nell'ambiente radice (@proprieta). Nel ramo di
scrittura, `entry(...).or_insert_with` gestisce anche il caso in cui un altro
thread abbia inserito lo stesso nome tra il rilascio del lock di lettura e
l'acquisizione di quello di scrittura: vince il primo, e tutti ricevono il suo
`Arc`.

== Uguaglianza <uguaglianza>

Il linguaggio ha due nozioni di uguaglianza, e il codice Rust ne ha una terza
su cui la seconda si appoggia.

=== `PartialEq` per `LispExp`

È scritto a mano, variante per variante:

#ref-table(
  columns: (3.2cm, 1fr),
  header: ([Coppia], [Quando sono uguali]),
  [`Cons`, `Cons`], [Confronto strutturale, #emph[iterativo] lungo i `cdr`: un ciclo avanza sulle due catene, si ferma con `true` se le due celle correnti sono lo stesso `Arc` (`Arc::ptr_eq`, che evita anche di riscorrere una coda condivisa), con `false` al primo `car` diverso. I `car` sono confrontati con una chiamata ricorsiva.],
  [`Lambda`, `Lambda`], [Solo se sono la stessa chiusura (`Arc::ptr_eq`). Due chiusure scritte uguali sono diverse: confrontarle strutturalmente vorrebbe dire confrontare gli ambienti catturati.],
  [`Form`, `Vector`, `Map`], [Confronto strutturale degli elementi (ricorsivo).],
  [`Number`], [`f64 ==`: quindi `NaN` non è uguale nemmeno a sé stesso.],
  [`Symbol`, `String`], [Confronto del testo.],
  [`Atom`], [Stesso `Arc` (`SharedAtom` confronta con `Arc::ptr_eq`).],
  [`Fiber`], [`SharedFiber` confronta gli indirizzi dei due involucri con `std::ptr::eq` #rapporto("INT-8").],
  [`Primitive`], [Stesso puntatore a funzione.],
  [varianti diverse], [Mai uguali: `(1 2)` come `Form` e come `Cons` sono diversi.],
)

=== `eq`, `eql` ed `equal`

#ref-table(
  columns: (2.4cm, 1fr),
  header: ([Primitiva], [Semantica]),
  [`equal`], [È `PartialEq`: il confronto strutturale profondo descritto sopra.],
  [`eq`, `eql`], [Identità. Numeri e simboli si confrontano per valore; `nil` con `nil` (in entrambe le forme, simbolo e `()`); forme, cons, vettori, mappe e stringhe sono `eq` solo se sono #emph[la stessa] allocazione (`Arc::ptr_eq`). Per le altre varianti (lambda, atomi, fiber, primitive) non c'è un ramo, e il risultato è `nil` anche per un oggetto confrontato con sé stesso #rapporto("INT-7").],
  [`=`, `<`, …], [Solo numeri; accettano un numero qualunque di argomenti e controllano l'intera catena (`compare_chain`).],
)

#ref-table(
  columns: (1fr, auto, auto),
  header: ([Espressione], [`eq`], [`equal`]),
  [`1` e `1.0`], [`t`], [`t`],
  [`'a` e `'a`], [`t`], [`t`],
  [`"ab"` e `"ab"` lette separatamente], [`nil`], [`t`],
  [`(list 1 2)` e `(list 1 2)`], [`nil`], [`t`],
  [`x` e `x`, con `x` una lista], [`t`], [`t`],
  [il valore di `(spawn …)` e `nil`], [`t`], [`nil`: una forma vuota e il simbolo `nil` sono varianti diverse #rapporto("INT-18")],
)

== Stampa

`Debug` è scritto a mano e produce sintassi Lisp: è ciò che finisce nell'area
dei messaggi, nei log e nei risultati di `M-:`.

#ref-table(
  columns: (2.4cm, 1fr),
  header: ([Variante], [Come si stampa]),
  [`Form`, `Cons`], [`(a b c)`; una lista puntata `(a b . c)`. Le due rappresentazioni si stampano allo stesso modo: la distinzione è interna.],
  [`Vector`, `Map`], [`[a b]`, `{chiave valore}`.],
  [`Number`], [Come intero se è finito, intero e in valore assoluto sotto $10^15$ (`3`, `-42`); altrimenti con il formato di `f64` (`1.5`, `1e300`, `inf`).],
  [`String`], [Tra virgolette, con gli escape di Rust: `"a\nb"`.],
  [`Lambda`], [`#<lambda (x &optional y &rest z)>`.],
  [`Primitive`, `Atom`], [`#<primitive>`, `#<atom>`. L'atomo non mostra il contenuto: è l'unico valore che può contenere sé stesso, e stamparlo potrebbe non finire.],
  [`Fiber`], [`#<fiber>`, `#<fiber done>`, oppure `#<fiber running>` se il lock è occupato (la stampa usa `try_read` e non aspetta).],
)

`lisp_display` (in `base/mod.rs`) è invece la conversione «per le persone»,
usata da `format` con `%s`, da `message` e da `insert`: una stringa è il suo
testo senza virgolette, un simbolo il suo nome, un numero passa da
`format_number`, tutto il resto come lo stampa `Debug`. `format_number` stampa
come intero ogni numero finito e intero, senza la soglia di $10^15$ di `Debug`
#rapporto("INT-14").

== Liberare una lista lunga

Una lista di un milione di elementi è una catena di un milione di `Arc`.
Lasciata alla distruzione automatica, ogni cella libererebbe la propria coda
dentro il proprio `drop`, e la profondità della ricorsione sarebbe la lunghezza
della lista: lo stack si esaurirebbe e il processo terminerebbe. `ConsCell` ha
quindi un `Drop` che scollega la catena in un ciclo:

#estratto("types.rs")[
```rust
impl<T: LispContext> Drop for ConsCell<T> {
    fn drop(&mut self) {
        let mut next = std::mem::replace(&mut self.cdr, LispExp::nil());
        while let LispExp::Cons(cell) = next {
            match Arc::try_unwrap(cell) {
                // l'ultimo proprietario: stacca la coda prima che la cella muoia
                Ok(mut owned) => {
                    next = std::mem::replace(&mut owned.cdr, LispExp::nil())
                }
                // qualcun altro tiene viva la coda: ci penserà lui
                Err(_) => break,
            }
        }
    }
}
```
]

#canvas(height: 2.9cm)[
  #node(1.5cm, 0.6cm, 1.8cm, 0.6cm, [cella 1], fill: warm.lighten(85%))
  #node(4.3cm, 0.6cm, 1.8cm, 0.6cm, [cella 2])
  #node(7.1cm, 0.6cm, 1.8cm, 0.6cm, [cella 3])
  #node(9.9cm, 0.6cm, 1.8cm, 0.6cm, [cella 4])
  #carrow((2.4cm, 0.6cm), (3.4cm, 0.6cm), colour: dim, dash: "dashed")
  #carrow((5.2cm, 0.6cm), (6.2cm, 0.6cm), colour: dim, dash: "dashed")
  #carrow((8.0cm, 0.6cm), (9.0cm, 0.6cm))
  #node(13.6cm, 1.5cm, 2.8cm, 0.6cm, [`b` (un'altra variabile)], fill: paper)
  #carrow((12.2cm, 1.5cm), (10.6cm, 0.9cm))
  #note(0.2cm, 1.5cm)[`drop` della cella 1: stacca la 2 (unico proprietario)\ e la libera, stacca la 3 e la libera; la 4 è tenuta viva\ da `b`: `try_unwrap` fallisce e il ciclo si ferma]
]

Il ciclo scende solo lungo i `cdr`. Una lista annidata nei `car` (ogni
elemento è la lista successiva) viene ancora liberata per ricorsione
#rapporto("INT-4").

== Sintassi e dati: `Form` contro `Cons` <sintassi-dati>

La distinzione più importante del modello è tra le liste che sono #emph[codice]
e quelle che sono #emph[dati].

#canvas(height: 4.4cm)[
  #node(2.6cm, 0.6cm, 4.2cm, 0.8cm, [`(+ 1 (* 2 3))` nel sorgente], fill: paper)
  #arrow((2.6cm, 1.0cm), (2.6cm, 1.75cm), label: [lettore], label-dx: 5pt, label-dy: -5pt)
  #node(2.6cm, 2.25cm, 4.2cm, 0.9cm, [`Form[+, 1, Form[*, 2, 3]]`\ vettori: accesso per indice])
  #note(0.4cm, 3.1cm)[ciò che `eval` smista: la testa\ è `list[0]`, gli argomenti `list[1..]`]

  #node(12.2cm, 0.6cm, 4.4cm, 0.8cm, [`'(1 2 3)` oppure `(list 1 2 3)`], fill: paper)
  #arrow((12.2cm, 1.0cm), (12.2cm, 1.75cm), label: [`quote` / primitive], label-dx: 5pt, label-dy: -5pt)
  #node(10.6cm, 2.25cm, 1.1cm, 0.7cm, [1 | •])
  #node(12.2cm, 2.25cm, 1.1cm, 0.7cm, [2 | •])
  #node(13.8cm, 2.25cm, 1.1cm, 0.7cm, [3 | `nil`])
  #carrow((11.0cm, 2.25cm), (11.6cm, 2.25cm))
  #carrow((12.6cm, 2.25cm), (13.2cm, 2.25cm))
  #note(10.0cm, 3.1cm)[ciò che vedono `car`, `cdr`, `mapcar`:\ una catena di `ConsCell`]

  #carrow((4.8cm, 2.0cm), (9.9cm, 2.0cm), label: [`form_to_data` (alla lettura, solo sotto `quote`)], dy: -8pt)
  #carrow((9.9cm, 2.55cm), (4.8cm, 2.55cm), label: [`data_to_form` (quando un dato viene valutato)], dy: 8pt, colour: accent)
]

Il lettore produce `Form` perché il valutatore accede alle liste di codice per
posizione: un vettore rende immediati `list[0]` e `list[1..]`. Le liste
#emph[valori] sono catene di `Cons`, come in ogni Lisp: `car` e `cdr` costano
O(1) e le code si possono condividere. Due funzioni convertono da una
rappresentazione all'altra.

=== `form_to_data`

#estratto("parser.rs")[
```rust
pub fn form_to_data<T: LispContext>(exp: &LispExp<T>) -> LispExp<T> {
    match exp {
        LispExp::Form(items) =>
            LispExp::proper_list(items.iter().map(form_to_data).collect()),
        LispExp::Vector(items) =>
            LispExp::vec(items.iter().map(form_to_data).collect()),
        LispExp::Map(m) =>
            LispExp::map(m.iter().map(|(k, v)| (k.clone(), form_to_data(v))).collect()),
        other => other.clone(),
    }
}
```
]

La chiama il lettore, una volta sola, su ciò che sta sotto un `'`: `'(1 (2 3))`
è già una catena di cons quando `quote` la restituisce, e la conversione non si
ripete a ogni valutazione. Una lista puntata scritta nel sorgente è già una
`Cons` (il lettore la costruisce con `improper_list`) e passa invariata.

=== `data_to_form`

#estratto("utils.rs")[
```rust
pub(super) fn data_to_form<T>(exp: &LispExp<T>) -> Result<LispExp<T>, EvalError<T>> {
    match exp {
        LispExp::Cons(_) => {
            let (items, tail) = exp.split_list();
            if !tail.is_nil() { return Err(EvalError::UnvalidFunctionCall); }
            // (quote X) costruita a mano: X deve restare un dato
            if items.len() == 2
                && matches!(&items[0], LispExp::Symbol(s) if s.as_str() == "quote")
            {
                return Ok(LispExp::form(vec![items[0].clone(), items[1].clone()]));
            }
            let mut form = Vec::with_capacity(items.len());
            for item in &items { form.push(data_to_form(item)?); }
            Ok(LispExp::form(form))
        }
        LispExp::Vector(items) => /* stessa cosa, elemento per elemento */,
        other => Ok(other.clone()),
    }
}
```
]

Fa il percorso inverso quando un #emph[dato] viene valutato: `(eval (list '+ 1 2))`
e ogni espansione di macro, che produce dati, passano da qui. Tre dettagli:

- una lista puntata non può diventare codice e dà `UnvalidFunctionCall`;
- `(quote X)` costruita a mano viene ricostruita senza convertire `X`, perché
  `quote` deve restituire `X` come dato; convertirlo lo trasformerebbe in
  sintassi, e `quote` restituirebbe un nodo `Form` come valore;
- le mappe non vengono convertite: i loro valori restano dati.

#caution[
  Una primitiva che restituisce una lista a Lisp deve usare
  `LispExp::proper_list` (dati), mai `LispExp::form` (sintassi). `LispExp::form`
  è riservato al lettore, alle espansioni di macro e a `data_to_form`. Vale
  anche al contrario: `(funcall '(lambda (x) x) 3)` non funziona, perché una
  lista citata è un dato e non una chiusura; una chiusura nasce solo valutando
  la forma `lambda`.
]

== Le strutture di `types.rs` <tipi>

#ref-table(
  columns: (auto, 1fr),
  header: ([Tipo], [Ruolo]),
  [`ConsCell { car, cdr }`], [Una cella di lista; `Drop` iterativo lungo i `cdr`.],
  [`ConsIter`], [L'iteratore restituito da `iter()`: tiene un cursore (`LispExp`) e produce i `car` finché il cursore è una `Cons`.],
  [`Lambda`], [`params` (obbligatori), `optionals` (`&optional`), `rest` (`&rest`), `body: Vec<LispExp>`, `env: Arc<Env>` catturato, `doc: Option<Arc<String>>`. È la funzione di `defun`, la chiusura di `lambda` e la macro di `defmacro`: cambiano solo lo spazio dei nomi in cui viene legata e il modo in cui è chiamata.],
  [`SharedAtom`], [`Arc<RwLock<LispExp>>`. Si confronta per identità.],
  [`Frame`], [Un blocco sospeso di una fiber: `Body { forms, from, env }` (il resto di un corpo) o `While { condition, forms, from, env }` (il resto di un'iterazione, poi il ciclo).],
  [`FiberState`], [`pending: Vec<Frame>` (il più interno per primo) e `is_done`.],
  [`SharedFiber`], [`Arc<RwLock<FiberState>>`: più thread possono avere la stessa fiber, uno solo la esegue.],
  [`LispPrimitive<T>`], [`fn(&[LispExp<T>], Arc<Env<T>>, &T) -> Result<LispExp<T>, EvalError<T>>`. Riceve gli argomenti già valutati, l'ambiente del #emph[chiamante] e il contesto.],
)

#canvas(height: 3.3cm)[
  #node(4.0cm, 1.55cm, 7.6cm, 2.7cm, [], fill: white)
  #note(0.4cm, 0.35cm)[*`Lambda`*, costruita da]
  #note(0.4cm, 0.72cm)[`(defun f (a &optional b &rest c) "doc" ...)`]
  #note(0.6cm, 1.17cm)[`params`: `["a"]` · `optionals`: `["b"]` · `rest`: `Some("c")`]
  #note(0.6cm, 1.57cm)[`body`: le forme dopo la docstring]
  #note(0.6cm, 1.97cm)[`doc`: `Some("doc")`]
  #note(0.6cm, 2.37cm)[`env`: l'ambiente in cui `defun` è stata valutata]
  #node(12.9cm, 2.45cm, 4.6cm, 0.8cm, [`Env` di definizione\ (di solito la radice)], fill: paper)
  #carrow((7.8cm, 2.45cm), (10.6cm, 2.45cm), label: [cattura], dy: -7pt)
  #note(8.3cm, 0.4cm)[ogni chiamata crea `Env::new_child(&lambda.env)`:\ lo scope è quello della definizione, non del chiamante]
]

// ===========================================================================
= Il lettore <lettore>
// ===========================================================================

Il lettore trasforma il testo in un albero di `LispExp`. È fatto di due parti
che collaborano: un #emph[lexer] a stati, che legge un carattere alla volta e
produce #emph[token], e un #emph[parser] a discesa ricorsiva, che legge i token e
costruisce le espressioni. Entrambe stanno in `parser.rs`, nella struttura
`Parser`.

== L'interfaccia

```rust
let mut parser = Parser::new(testo);     // non legge ancora nulla
let prima = parser.next()?;              // la prima espressione completa
let seconda = parser.next()?;            // la successiva, e così via
// alla fine del testo: Err(ParserError::VoidExp)
```

`next()` restituisce #emph[una] espressione completa per chiamata. Chi deve
leggere un file intero può chiamarla in un ciclo finché riceve `VoidExp`,
oppure avvolgere il testo in `(progn …)` e leggere una sola espressione, che è
ciò che fa `eval_file` nell'editor. `eval-string` legge e valuta soltanto la
prima espressione della stringa.

== Lo stato del `Parser`

#estratto("parser.rs")[
```rust
pub struct Parser<'source> {
    source: Peekable<Chars<'source>>, // i caratteri, uno di anticipo
    token: String,                    // il token in costruzione
    current_token: Token,             // il token di anticipo del parser
    parens_stack: Vec<Token>,         // le aperture non ancora chiuse
    lexer_state: ParserLexerState,    // dove si trova il lexer in un token
}
```
]

#ref-table(
  columns: (3.2cm, 1fr),
  header: ([Token], [Testo che lo produce]),
  [`LParen`, `RParen`], [`(` e `)`],
  [`LSquared`, `RSquared`], [`[` e `]`],
  [`LBracket`, `RBracket`], [`{` e `}`],
  [`Quote`, `BackQuote`], [`'` e #raw("`")],
  [`Comma`, `CommaAt`], [`,` e `,@`],
  [`Dot`], [un `.` isolato],
  [`Number(f64)`, `String`, `Symbol`], [gli atomi],
  [`Void`], [la fine dei token (prodotto da `advance_token`, non dal lexer)],
  [`Uninitialized`], [lo stato iniziale di `current_token`, prima della prima lettura],
)

== Il lexer

`next_token` guarda il carattere successivo con `peek()`, decide che cosa fare
in base allo stato corrente e, nella maggior parte dei casi, lo consuma con
`next()` in fondo al ciclo. Quando un carattere chiude il token in corso senza
farne parte --- una parentesi dopo un simbolo, per esempio --- lo stato torna a
`Default` #emph[senza] consumarlo: la chiamata successiva lo troverà di nuovo e
lo tratterà come il primo carattere di un nuovo token.

#canvas(height: 6.4cm)[
  #node(8.1cm, 0.5cm, 2.6cm, 0.8cm, [`Default`], fill: paper)

  #node(1.2cm, 2.6cm, 2.2cm, 0.8cm, [`InComment`])
  #node(3.7cm, 2.6cm, 2.2cm, 0.8cm, [`InSymbol`])
  #node(6.2cm, 2.6cm, 2.2cm, 0.8cm, [`InString`])
  #node(8.8cm, 2.6cm, 2.2cm, 0.8cm, [`InNumber`])
  #node(12.0cm, 2.6cm, 3.4cm, 0.8cm, [`InNumberMinusStart`])
  #node(15.3cm, 2.6cm, 2.4cm, 0.8cm, [`InDotStart`])

  #carrow((7.0cm, 0.8cm), (1.6cm, 2.2cm), label: [`;`], dx: -10pt)
  #carrow((7.3cm, 0.9cm), (3.9cm, 2.2cm), label: [altro], dx: -12pt)
  #carrow((7.8cm, 0.9cm), (6.4cm, 2.2cm), label: [`"`], dx: -6pt)
  #carrow((8.3cm, 0.9cm), (8.7cm, 2.2cm), label: [cifra], dx: 12pt)
  #carrow((9.0cm, 0.8cm), (11.6cm, 2.2cm), label: [`-`], dx: 8pt)
  #carrow((9.4cm, 0.6cm), (15.0cm, 2.2cm), label: [`.`], dx: 10pt)

  #node(6.2cm, 4.5cm, 2.6cm, 0.8cm, [`InStringSlash`])
  #carrow((5.8cm, 3.0cm), (5.8cm, 4.1cm), label: [`\`], dx: -8pt)
  #carrow((6.6cm, 4.1cm), (6.6cm, 3.0cm), label: [escape], dx: 16pt)

  #node(11.6cm, 4.5cm, 3.2cm, 0.8cm, [`InNumberAfterDot`])
  #carrow((9.0cm, 3.0cm), (10.8cm, 4.1cm), label: [`.`], dx: -8pt)
  #carrow((11.8cm, 3.0cm), (11.7cm, 4.1cm), label: [`.`], dx: 7pt)
  #carrow((15.2cm, 3.0cm), (12.6cm, 4.1cm), label: [cifra 0–8], dx: 18pt)

  #note(0.2cm, 5.3cm)[Ogni stato torna a `Default` quando emette il suo token. Da `InNumberMinusStart` una cifra porta a `InNumber`; dagli stati dei numeri\ una lettera porta a `InSymbol` (`1+`, `->`), tranne da `InNumberAfterDot`, che dà `NumberParseError` (`1.5x`). La tabella sotto è il riferimento completo.]
]

#ref-table(
  columns: (3.7cm, 1fr),
  header: ([Stato], [Carattere letto → azione]),
  [`Default`], [spazio, tab, `\n`, `\r` → si salta · `;` → `InComment` · `'` → `Quote` · #raw("`") → `BackQuote` · `,` → `Comma`, oppure `CommaAt` se seguita da `@` · `(` `[` `{` → empila l'apertura su `parens_stack` e la emette · `)` `]` `}` → disimpila: se l'apertura corrisponde emette la chiusura, altrimenti `UnbalancedRParen` (o `…RSquared`, `…RBracket`) · `"` → `InString` · `-` → `InNumberMinusStart` · cifra → `InNumber` · `.` → `InDotStart` · altro → `InSymbol`],
  [`InComment`], [`\n` → `Default` · ogni altro carattere si salta],
  [`InSymbol`], [spazio, tab, `\n` → emette `Symbol` e consuma il separatore · parentesi di ogni tipo → emette `Symbol` senza consumare · `;` → emette `Symbol`, poi `InComment` · `\r` → ignorato · altro → si accumula (anche `"`, `'`, `,`)],
  [`InString`], [`"` → emette `String` · `\` → `InStringSlash` · altro → si accumula, a capo compreso],
  [`InStringSlash`], [`\"` e `\\` → il carattere · `\n`, `\t`, `\r` → a capo, tab, ritorno · `\e` → ESC · `\0` → NUL · ogni altro `\x` → si accumulano #emph[entrambi] i caratteri; poi `InString`],
  [`InNumber`], [cifra → si accumula · `.` → `InNumberAfterDot` · separatore, parentesi o `;` → converte con `str::parse::<f64>` ed emette `Number` · altro → `InSymbol`],
  [`InNumberMinusStart`], [cifra → `InNumber` · `.` → `InNumberAfterDot` · separatore, parentesi o `;` → emette `Symbol("-")` · altro → `InSymbol`],
  [`InNumberAfterDot`], [cifra → si accumula · separatore, parentesi o `;` → converte ed emette `Number` · altro → `NumberParseError`],
  [`InDotStart`], [cifra da `0` a `8` → `InNumberAfterDot` · separatore, parentesi o `;` → emette `Dot` · altro, compreso `9` → `InSymbol` #rapporto("LET-4")],
  [fine dell'input], [`Default`, `InComment` → nessun token · `InSymbol`, `InNumberMinusStart` → emette `Symbol` #rapporto("LET-1") · `InNumber`, `InNumberAfterDot` → emette `Number` · `InDotStart` → emette `Dot` · `InString`, `InStringSlash` → `UnclosedString`],
)

Da queste regole discendono le convenzioni che contano quando si scrive Lisp
per rsedit:

#ref-table(
  columns: (auto, 1fr),
  header: ([Elemento], [Regola]),
  [Delimitatori], [Spazio, tabulazione, `\n`, parentesi di ogni tipo; `;` apre un commento fino a fine riga anche attaccato a un simbolo (`foo;commento`).],
  [Numeri], [Cifre con un punto facoltativo, eventualmente precedute da `-`: `42`, `-3`, `1.5`, `.5`. Una cifra seguita da lettere diventa un simbolo (`1+`); `1.5x` è un errore. Non esiste la notazione esponenziale: `1e20` è un simbolo.],
  [`-`], [Da solo, o seguito da lettere, è un simbolo (`-`, `->`, `-foo`); seguito da cifre è un numero.],
  [Stringhe], [Escape riconosciuti: `\"`, `\\`, `\n`, `\t`, `\r`, `\e` (ESC), `\0`. Ogni altro `\x` resta com'è, backslash compreso: così le espressioni regolari possono usare `\b`, `\s`, `\.` con un solo backslash.],
  [Simboli], [Qualunque sequenza di caratteri che non siano delimitatori: `*scratch*`, `C-x`, `string=`, `&rest`, `:keyword`.],
)

== Il parser

Il parser tiene un solo token di anticipo, `current_token`, e lo sostituisce con
il successivo chiamando `advance_token`, che trasforma la fine dei token in
`Token::Void`. La grammatica, in forma compatta:

```text
espr    ::= atomo | lista | vettore | mappa
          | "'" espr | "`" espr | "," espr | ",@" espr
lista   ::= "(" espr* ")"  |  "(" espr+ "." espr ")"
vettore ::= "[" espr* "]"
mappa   ::= "{" (chiave espr)* "}"         chiave ::= simbolo | stringa
atomo   ::= numero | stringa | simbolo
```

Ogni regola è una funzione. `next` guarda `current_token` e sceglie:

#estratto("parser.rs")[
```rust
pub fn next<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
    match self.current_token.clone() {
        Token::Symbol(s) => { self.advance_token()?; Ok(LispExp::symbol(s)) }
        Token::String(s) => { self.advance_token()?; Ok(LispExp::string(s)) }
        Token::Number(n) => { self.advance_token()?; Ok(LispExp::number(n)) }
        Token::LParen    => { self.advance_token()?; self.parse_list() }
        Token::LSquared  => { self.advance_token()?; self.parse_vector() }
        Token::LBracket  => { self.advance_token()?; self.parse_map() }
        Token::Uninitialized => { self.advance_token()?; self.next() }
        Token::Quote => {
            self.advance_token()?;
            let quoted = self.next()?;
            Ok(LispExp::form(vec![LispExp::symbol("quote"), form_to_data(&quoted)]))
        }
        Token::BackQuote => /* Form[backquote, next()] */,
        Token::Comma     => /* Form[unquote, next()] */,
        Token::CommaAt   => /* Form[unquote-splicing, next()] */,
        Token::Void => Err(ParserError::VoidExp),
        _ => unreachable!(...),    // Dot, RParen, RSquared, RBracket
    }
}
```
]

L'ultimo ramo raccoglie i token che non possono cominciare un'espressione: un
`.` fuori dalla coda di una lista, o una chiusura subito dopo un prefisso come
in `'(a ')` #rapporto("LET-2").

=== Liste, vettori e mappe

#estratto("parser.rs")[
```rust
fn parse_list<T: LispContext>(&mut self) -> Result<LispExp<T>, ParserError> {
    let mut list = vec![];
    while self.current_token != Token::Void {
        match self.current_token {
            Token::RParen => { self.advance_token()?; return Ok(LispExp::form(list)); }
            Token::Dot => {
                if list.is_empty() { return Err(ParserError::UnexpectedDot); }
                self.advance_token()?;
                if self.current_token == Token::RParen || self.current_token == Token::Void {
                    return Err(ParserError::UnexpectedDot);
                }
                let tail = self.next()?;
                if self.current_token != Token::RParen {
                    return Err(ParserError::MalformedDottedList);
                }
                self.advance_token()?;
                return Ok(LispExp::improper_list(list, tail));
            }
            _ => list.push(self.next()?),
        }
    }
    Err(ParserError::UnclosedList)
}
```
]

Una lista normale diventa una `Form`; una lista puntata `(a b . c)` diventa
direttamente una catena di `Cons` che termina in `c`, perché non è codice
valutabile. `parse_vector` è lo stesso ciclo senza il punto. `parse_map`
alterna due fasi: in quella della chiave accetta solo un simbolo o una stringa
(`InvalidMapKey` altrimenti) e ne conserva il #emph[testo]; in quella del valore
legge un'espressione qualsiasi, e una `}` al posto del valore è
`MapKeyMissingValue`. Due chiavi uguali tengono l'ultimo valore.

=== Un esempio completo

La figura segue la lettura di `'(a [1 2] "s")`: in alto la sequenza dei token
che il lexer produce, in basso l'albero delle chiamate del parser e ciò che
ciascuna restituisce.

#canvas(height: 7.1cm)[
  #let tok(i, t) = node(0.8cm + i * 1.55cm, 0.4cm, 1.45cm, 0.55cm, t, size: 7.2pt, fill: paper)
  #tok(0, [`Quote`])
  #tok(1, [`LParen`])
  #tok(2, [`Symbol`])
  #tok(3, [`LSquared`])
  #tok(4, [`Num 1`])
  #tok(5, [`Num 2`])
  #tok(6, [`RSquared`])
  #tok(7, [`String`])
  #tok(8, [`RParen`])
  #tok(9, [`Void`])

  #node(2.6cm, 1.6cm, 4.2cm, 0.65cm, [`next()` su `Quote`])
  #node(2.7cm, 2.8cm, 4.8cm, 0.65cm, [`next()` su `LParen` → `parse_list`])
  #carrow((2.6cm, 1.93cm), (2.6cm, 2.47cm))
  #node(1.5cm, 4.1cm, 2.6cm, 0.65cm, [`next()`: `a`])
  #node(5.0cm, 4.1cm, 3.6cm, 0.65cm, [`next()` → `parse_vector`])
  #node(8.8cm, 4.1cm, 2.6cm, 0.65cm, [`next()`: `"s"`])
  #carrow((2.0cm, 3.13cm), (1.6cm, 3.77cm))
  #carrow((3.1cm, 3.13cm), (4.6cm, 3.77cm))
  #carrow((4.5cm, 2.95cm), (8.4cm, 3.77cm))
  #node(4.0cm, 5.3cm, 1.6cm, 0.6cm, [`1`])
  #node(6.0cm, 5.3cm, 1.6cm, 0.6cm, [`2`])
  #carrow((4.6cm, 4.43cm), (4.1cm, 5.0cm))
  #carrow((5.4cm, 4.43cm), (5.9cm, 5.0cm))
  #note(6.9cm, 4.75cm)[`RSquared`: restituisce `Vector[1, 2]`]
  #note(10.3cm, 2.55cm)[`RParen`: `parse_list` restituisce\ `Form[a, Vector[1, 2], "s"]`]

  #node(13.6cm, 1.6cm, 4.6cm, 1.2cm, [risultato del `Quote`: `Form[quote, ●]`,\ con ● la catena di dati `(a [1 2] "s")`], fill: paper)
  #carrow((4.7cm, 1.6cm), (11.3cm, 1.6cm), label: [`form_to_data`], dy: -7pt)
  #note(0.2cm, 6.0cm)[In alto i token, nell'ordine in cui il lexer li emette. Ogni chiamata a `next()` consuma il proprio token con `advance_token` e lascia in `current_token`\ il primo token che non le appartiene: è così che `parse_list` vede la `RParen` dopo che `parse_vector` ha consumato la `RSquared`.]
]

=== Gli errori del lettore

#ref-table(
  columns: (4.8cm, 1fr),
  header: ([`ParserError`], [Quando]),
  [`UnbalancedRParen`, `…RSquared`, `…RBracket`], [Una chiusura che non corrisponde all'ultima apertura sulla `parens_stack`, o una chiusura senza aperture: `)`, `(]`, `[}`.],
  [`UnclosedList`, `UnclosedVector`, `UnclosedMap`], [Il testo finisce prima della chiusura (in uno stato in cui il lexer lo può dire, vedi #rapporto("LET-1")).],
  [`UnclosedString`], [Il testo finisce dentro una stringa.],
  [`NumberParseError(testo)`], [Il testo di un numero non si converte in `f64`, o un numero con il punto è seguito da una lettera: `1.5x`.],
  [`UnexpectedDot`], [Un punto all'inizio di una lista `( . a)` o subito prima della chiusura `(a . )`.],
  [`MalformedDottedList`], [Più di un elemento dopo il punto: `(a . b c)`.],
  [`InvalidMapKey`, `MapKeyMissingValue`], [Una chiave che non è un simbolo o una stringa; una chiave senza valore.],
  [`VoidExp`], [Il testo è finito: non è un errore vero, è il modo in cui chi legge in un ciclo capisce di aver letto tutto.],
)

#caution[
  I prefissi `'` e #raw("`") sono trasformati dal lettore. Una virgola dentro
  una forma #emph[citata] (`',x`) è già un dato quando l'espansore del
  backquote la vede, e non viene espansa. Il commento in `defcommand`
  (`core/lisp/commands.lisp`) mostra come aggirare il problema.
]

// ===========================================================================
= Gli ambienti <ambienti>
// ===========================================================================

Un ambiente associa nomi a valori. È l'unico posto in cui un simbolo acquista un
significato: valutare `x` vuol dire cercare `x` in un ambiente, chiamare `(f 1)`
vuol dire cercare `f` in un ambiente.

== La struttura di `Env`

#estratto("environment.rs")[
```rust
pub struct Env<T: LispContext> {
    pub variables:  RwLock<HashMap<String, LispExp<T>>>,
    pub functions:  RwLock<HashMap<String, LispExp<T>>>,
    pub macros:     RwLock<HashMap<String, LispExp<T>>>,
    pub properties: Option<Box<PropertyTable<T>>>,  // solo nella radice
    pub parent:     Option<Arc<Env<T>>>,
}
```
]

Un ambiente ha #emph[tre spazi dei nomi] separati, come Emacs Lisp: un simbolo
può essere insieme una variabile, una funzione e una macro senza che i tre
legami interferiscano. `(setq sq 5)` e `(defun sq () 6)` convivono: `sq` vale
5 e `(sq)` vale 6. Una macro non è una funzione: `funcall` e `functionp` non la
vedono. Ogni tabella ha il proprio `RwLock`, così due thread possono leggere lo
stesso ambiente insieme e una scrittura blocca una sola tabella.

Gli ambienti formano una catena verso la radice. Ci sono due costruttori:

- `Env::new_root()` crea la radice, l'unico ambiente con la tabella delle
  proprietà; la crea `bootstrap_vm`, una volta per interprete;
- `Env::new_child(&genitore)` crea un anello vuoto, senza tabella delle
  proprietà. Viene chiamato a ogni chiamata di funzione, a ogni `let` e `let*`,
  in ogni `dolist` e `dotimes`, nel gestore di `condition-case`, in una fiber e
  in uno `spawn`: per questo è tenuto il più leggero possibile.

#canvas(height: 5.6cm)[
  #node(3.0cm, 0.6cm, 5.6cm, 1.15cm, [*radice* (`bootstrap_vm`)\ primitive, `defun` e `defvar` globali,\ tabella delle proprietà], fill: paper)
  #node(3.0cm, 2.6cm, 5.6cm, 0.9cm, [chiamata a `f`: `new_child(&f.env)`\ parametri di `f`])
  #node(3.0cm, 4.4cm, 5.6cm, 0.9cm, [`(let ((x 1)) ...)` dentro `f`\ `x`])
  #arrow((3.0cm, 2.15cm), (3.0cm, 1.2cm), label: [`parent`], label-dx: 5pt, label-dy: -5pt)
  #arrow((3.0cm, 3.95cm), (3.0cm, 3.05cm), label: [`parent`], label-dx: 5pt, label-dy: -5pt)

  #node(11.8cm, 1.3cm, 6.6cm, 1.5cm, [*ricerca* (`get_variable`, `get_function`, `get_macro`):\ dal più interno verso la radice; il lock di ogni\ anello è rilasciato prima di passare al genitore], fill: accent.lighten(90%))
  #node(11.8cm, 3.7cm, 6.6cm, 1.5cm, [*scrittura*: `set_variable` lega #emph[qui];\ `update_variable` aggiorna il legame più vicino;\ `set_root_variable` lega nella radice], fill: accent.lighten(90%))
  #carrow((8.5cm, 4.3cm), (5.9cm, 4.4cm), colour: accent)
  #carrow((8.5cm, 1.3cm), (5.9cm, 1.0cm), colour: accent)
]

== La ricerca

Le tre ricerche sono la stessa funzione, parametrizzata sullo spazio dei nomi:

#estratto("environment.rs")[
```rust
fn lookup(&self, namespace: Namespace, name: &str) -> Option<LispExp<T>> {
    let mut env = self;
    loop {
        let found = env.table(namespace).read().unwrap().get(name).cloned();
        if found.is_some() { return found; }       // il lock è già rilasciato qui
        env = env.parent.as_deref()?;              // radice superata: None
    }
}
```
]

Il ciclo è iterativo e prende #emph[un lock alla volta]: il lock di lettura di
un anello viene rilasciato, alla fine dell'istruzione, prima che si passi al
genitore. Tenere un lock su tutta la catena insieme imporrebbe un ordine di
acquisizione che il resto dell'editor non rispetta, e un thread che scrive in un
anello esterno potrebbe bloccarsi contro uno che legge da un anello interno.
Il valore trovato è clonato, cioè restituito come un nuovo `Arc` sullo stesso
dato: chi lo riceve non tiene nulla dell'ambiente.

== La scrittura

Ci sono tre modi di scrivere una variabile, e le forme speciali scelgono quale
usare (@come-lega):

#ref-table(
  columns: (4.6cm, 1fr),
  header: ([Metodo], [Che cosa fa]),
  [`set_variable(n, v)`], [Lega `n` nell'ambiente #emph[su cui è chiamato], anche se un anello esterno ha già un legame con lo stesso nome: il nuovo legame lo nasconde.],
  [`update_variable(n, v)`], [Cerca il legame più vicino e lo sostituisce, restituendo `true`; se nessun anello lo ha, non fa nulla e restituisce `false`. È ricorsivo lungo la catena: controlla con un lock di lettura se l'anello contiene il nome, poi prende il lock di scrittura per sostituirlo.],
  [`set_root_variable(n, v)`], [Risale fino alla radice e lega lì.],
)

`set_function` e `set_macro` legano sempre nell'anello su cui sono chiamati.

== Come lega ciascuna forma <come-lega>

#ref-table(
  columns: (auto, 1fr),
  header: ([Forma], [Dove scrive]),
  [`setq`], [`update_variable`: aggiorna il legame esistente più vicino. Se non ne trova, crea il legame #emph[nell'ambiente corrente] (`set_variable`). Restituisce l'ultimo valore assegnato.],
  [`let`], [Valuta tutti i valori nell'ambiente esterno, poi li lega in un figlio nuovo.],
  [`let*`], [Lega uno alla volta nel figlio, così ogni valore vede i precedenti.],
  [`defvar`], [Lega nell'ambiente #emph[corrente] solo se la variabile non è visibile da lì. La docstring, se c'è, va sempre nelle proprietà del simbolo.],
  [`defconst`], [Come `defvar`, ma assegna sempre.],
  [`defun`, `defmacro`], [Legano nell'ambiente corrente, nello spazio delle funzioni o delle macro.],
  [`dolist`, `dotimes`], [Creano un figlio con la variabile del ciclo e la aggiornano a ogni giro.],
  [parametri di una chiamata], [`bind_lambda_args` lega con `set_variable` nell'ambiente nuovo della chiamata.],
  [primitive come `add-to-list`], [Scrivono con `set_variable` nell'ambiente che ricevono, cioè quello del chiamante.],
)

Poiché tutte queste forme scrivono nell'ambiente #emph[in cui sono valutate],
ciò che accade dentro il corpo di una funzione resta locale alla chiamata:

- `(setq nuova 1)` su una variabile mai dichiarata crea un legame nell'ambiente
  della chiamata, che sparisce al ritorno #rapporto("INT-3");
- `defun`, `defvar` e `defmacro` eseguiti dentro una funzione definiscono
  nomi visibili solo durante quella chiamata;
- `(add-to-list 'lista x)` chiamato in una funzione lega una nuova lista
  nell'ambiente della chiamata #rapporto("INT-15").

Per modificare lo stato globale da una funzione, la variabile deve esistere già
nella radice: `setq` la trova risalendo la catena e la aggiorna lì. È così che
funzionano i moduli in `core/lisp/`: dichiarano le variabili a livello di file,
dove l'ambiente corrente è la radice.

== Lo scope è lessicale <scope>

Una `Lambda` ricorda l'ambiente in cui è stata creata, e la sua chiamata crea un
figlio di #emph[quell']ambiente, non di quello del chiamante. Le variabili
globali sono visibili ovunque perché stanno nella radice, in cima a ogni
catena. Il test `test_lexical_vs_dynamic_scoping` fissa la scelta.

```lisp
(setq x 10)
(defun get-x () x)
(let ((x 99)) (get-x))   ; => 10 in rsedit, 99 in Emacs con scope dinamico
```

Il valore `10` si spiega seguendo le catene: il corpo di `get-x` viene valutato
in un figlio dell'ambiente di #emph[definizione], la radice, e da lì la ricerca
di `x` non passa mai dall'anello del `let`.

#canvas(height: 4.2cm)[
  #node(2.6cm, 0.5cm, 4.6cm, 0.75cm, [radice: `k`, `funcall`, ...], fill: paper)
  #node(2.6cm, 1.85cm, 4.6cm, 0.75cm, [`let`: `n = 0 → 1 → 2`])
  #node(2.6cm, 3.3cm, 4.6cm, 0.75cm, [chiamata della chiusura])
  #arrow((2.6cm, 1.47cm), (2.6cm, 0.88cm), label: [`parent`], label-dx: 5pt, label-dy: -5pt)
  #arrow((2.6cm, 2.92cm), (2.6cm, 2.23cm), label: [`parent`], label-dx: 5pt, label-dy: -5pt)
  #node(11.3cm, 1.85cm, 7.4cm, 0.75cm, [`Lambda { env: ●, body: (setq n (+ n 1)) }`])
  #carrow((7.6cm, 1.85cm), (4.9cm, 1.85cm), label: [`env`], dy: -7pt)
  #note(7.6cm, 2.6cm)[`(setq k (let ((n 0)) (lambda () (setq n (+ n 1)))))`\ `(funcall k) (funcall k)` → `2`: la chiusura aggiorna\ la `n` del `let`, che sopravvive finché la chiusura vive]
]

La figura mostra il caso opposto, una chiusura che #emph[vuole] vedere un
ambiente diverso dalla radice. La chiusura tiene in vita l'anello del `let`
attraverso il proprio campo `env`, e ogni chiamata ne crea un figlio:
`(setq n …)` risale la catena con `update_variable` e trova la `n` del `let`.
L'anello vive finché vive la chiusura: è il conteggio dei riferimenti degli
`Arc` a decidere quando liberarlo. Se l'anello contiene a sua volta la chiusura
(per esempio perché la si è assegnata a una variabile dello stesso `let`), i
due si tengono in vita a vicenda #rapporto("INT-6").

== Le proprietà dei simboli <proprieta>

`put` e `get` leggono e scrivono una tabella `simbolo → chiave → valore` che
esiste soltanto nella radice. Da qualunque anello, `property_owner` risale la
catena fino al primo ambiente che ha la tabella:

#estratto("environment.rs")[
```rust
fn property_owner(&self) -> &Env<T> {
    let mut env = self;
    loop {
        if env.properties.is_some() { return env; }
        match env.parent.as_deref() { Some(p) => env = p, None => return env }
    }
}
```
]

Le proprietà sono quindi globali e non hanno scope: una proprietà impostata
dentro un `let` resta dopo. L'editor le usa per i dati che appartengono a un
nome, per esempio la larghezza del tab di un modo, `(put 'rust-mode 'tab-width 4)`.
La docstring di una variabile è la proprietà `variable-documentation`
(la costante `VARIABLE_DOCUMENTATION`), letta da `variable-doc`.

== Letture tipizzate ed elenchi di nomi

Due metodi esistono perché l'host legge spesso impostazioni da Lisp:

- `number_at_least(nome, minimo)` restituisce il numero solo se è finito e
  almeno `minimo`, così una stringa, un negativo o un infinito non arrivano mai
  a `Duration::from_secs_f64`;
- `flag(nome, default)` distingue #emph[non legata] da `nil`: un modulo che non
  si è caricato non deve poter spegnere una funzionalità per omissione.

`variable_names`, `function_names` e `macro_names` raccolgono le chiavi di
ogni anello, dal corrente alla radice, poi le ordinano e tolgono i duplicati:
è ciò che restituiscono `all-variables`, `all-functions` e `all-macros`, e ciò
su cui si basa il completamento di `M-x` nell'editor.

// ===========================================================================
= Il valutatore <valutatore>
// ===========================================================================

Il valutatore sta in `eval.rs` ed è fatto di cinque funzioni che si chiamano
in quest'ordine: `eval` (il ciclo), `eval_step` (il permesso di sospendersi),
`eval_step_permitted` (il carburante e lo smistamento per tipo),
`eval_special_form_or_call_step` (le forme speciali) ed
`eval_macro_or_function_call_step` (le chiamate). Ognuna restituisce un
`EvalStep`: o un valore finito, o la forma successiva da valutare.

```rust
enum EvalStep<T: LispContext> {
    Done(LispExp<T>),                   // il passo ha prodotto il valore
    TailCall(LispExp<T>, Arc<Env<T>>),  // resta da valutare questa forma, qui
}
```

== Il trampolino <trampolino>

#estratto("eval.rs")[
```rust
pub fn eval<T: LispContext>(exp: &LispExp<T>, env: Arc<Env<T>>, ctx: &T)
    -> Result<LispExp<T>, EvalError<T>>
{
    let mut current_exp = exp.clone();
    let mut current_env = env;
    loop {
        match eval_step(&current_exp, current_env.clone(), ctx)? {
            EvalStep::Done(result) => return Ok(result),
            EvalStep::TailCall(next_exp, next_env) => {
                current_exp = next_exp;
                current_env = next_env;
            }
        }
    }
}
```
]

Un passo che sa che il proprio risultato è il valore di un'altra forma non la
valuta: la restituisce come `TailCall`, e il ciclo la valuta al giro
successivo, #emph[nello stesso frame Rust]. Così una chiamata in coda non
consuma stack (lo si è visto con `fatt` nel @panoramica).

#canvas(height: 6.5cm)[
  #node(2.0cm, 1.15cm, 3.2cm, 0.8cm, [`eval(exp, env, ctx)`], fill: paper)
  #carrow((2.0cm, 1.55cm), (2.0cm, 2.3cm))
  #node(2.0cm, 2.75cm, 3.2cm, 0.9cm, [`eval_step`\ prende il permesso di `yield`])
  #carrow((3.6cm, 2.75cm), (5.6cm, 2.75cm))
  #node(8.0cm, 2.75cm, 4.6cm, 0.9cm, [`eval_step_permitted`\ `consume_fuel(1)`, smista per tipo])
  #carrow((8.0cm, 3.2cm), (8.0cm, 4.1cm))
  #node(8.0cm, 4.65cm, 4.6cm, 1.0cm, [atomi, simboli, vettori, mappe →\ forme speciali → macro → funzioni])
  #carrow((10.3cm, 4.65cm), (12.4cm, 3.75cm))
  #carrow((10.3cm, 4.95cm), (12.4cm, 5.55cm))
  #node(14.2cm, 3.45cm, 3.2cm, 0.8cm, [`Done(valore)`], fill: paper)
  #node(14.0cm, 5.65cm, 3.6cm, 0.8cm, [`TailCall(forma, env)`], fill: accent.lighten(85%))
  #segment((15.8cm, 5.65cm), (16.3cm, 5.65cm), colour: accent)
  #segment((16.3cm, 5.65cm), (16.3cm, 0.35cm), colour: accent)
  #segment((16.3cm, 0.35cm), (2.0cm, 0.35cm), colour: accent)
  #carrow((2.0cm, 0.35cm), (2.0cm, 0.75cm), colour: accent)
  #note(8.6cm, 0.0cm, colour: accent)[il ciclo riparte con la nuova coppia]
  #carrow((14.2cm, 3.05cm), (14.2cm, 1.75cm))
  #note(13.0cm, 1.3cm)[`return Ok(valore)`]
]

Ogni altra valutazione annidata --- gli argomenti di una chiamata, le
condizioni, le forme non finali di un corpo --- chiama `eval` ricorsivamente, e
quindi apre un nuovo frame Rust. La tabella elenca tutte le posizioni di coda.

#ref-table(
  columns: (1fr, 1fr),
  header: ([In posizione di coda (`TailCall`)], [Valutato con una chiamata ricorsiva]),
  [l'ultima forma di `progn`, `let`, `let*`, `when`, `unless`, di una clausola di `cond`, del corpo di una funzione o di una chiusura chiamata in testa a una forma], [le forme precedenti di quei corpi],
  [i due rami di `if`], [la condizione di `if`, i test di `cond`],
  [l'ultima forma di `and` e di `or`], [le forme precedenti],
  [l'espansione di una macro], [il corpo della macro che produce l'espansione],
  [un dato valutato (`Cons`, dopo `data_to_form`)], [gli argomenti di ogni chiamata],
  [l'ultima forma di un gestore di `condition-case`], [la forma protetta di `condition-case`, il corpo di `catch` e di `unwind-protect`, ogni forma di `while`, `dolist`, `dotimes`, `prog1`, `prog2`],
  [una forma con una forma in testa, riscritta con il valore della testa], [la testa stessa],
)

#key[
  Una ricorsione in coda non consuma stack: una funzione che si richiama in
  coda 200 000 volte termina normalmente. Una ricorsione #emph[non] in coda
  consuma uno o più frame Rust per livello, e nessun contatore la limita: il
  carburante limita il numero di passi, non la profondità #rapporto("INT-2").
  `catch`, `condition-case` (sulla forma protetta) e `unwind-protect` valutano
  il corpo dentro il proprio frame, perché devono esserci quando l'errore
  risale, e quindi non sono trasparenti alla chiamata in coda.
]

== Il permesso di sospendersi

`eval_step` esiste per un solo motivo: gestire il permesso di `yield`, una
variabile per thread che dice se la valutazione che sta per cominciare è in una
posizione da cui una fiber può essere sospesa e ripresa (@fiber).

#estratto("eval.rs")[
```rust
fn eval_step<T: LispContext>(exp: &LispExp<T>, env: Arc<Env<T>>, ctx: &T)
    -> Result<EvalStep<T>, EvalError<T>>
{
    let permitted = take_yield_permission();       // lo prende e lo azzera
    let step = eval_step_permitted(exp, env, ctx, permitted);
    // una chiamata in coda continua la stessa istruzione: il permesso passa
    if permitted && matches!(step, Ok(EvalStep::TailCall(..))) {
        grant_yield_permission();
    }
    step
}
```
]

Il permesso è #emph[consumato] all'inizio di ogni passo: qualunque valutazione
annidata dentro quel passo parte senza. È #emph[restituito] quando il passo
finisce con una chiamata in coda, perché la forma che continua è la stessa
istruzione. Le posizioni che concedono il permesso sono poche e sono descritte
nel @fiber.

== Lo smistamento per tipo <smistamento>

`eval_step_permitted` addebita un'unità di carburante, prima di qualunque altra
cosa, e poi guarda la variante dell'espressione.

#estratto("eval.rs")[
```rust
ctx.consume_fuel(1)?;
match exp {
    String(_) | Number(_) | Atom(_) | Fiber(_) | Lambda(_) | Primitive { .. }
        => Ok(Done(exp.clone())),
    Symbol(s) => {
        if s == "nil" || s == "t" || s.starts_with(':') { return Ok(Done(exp.clone())); }
        env.get_variable(s).map(Done).ok_or(UnboundVariable(s.to_string()))
    }
    Cons(_) => Ok(TailCall(data_to_form(exp)?, env)),
    Form(list) if list.is_empty() => Ok(Done(LispExp::nil())),
    Form(list) => match &list[0] {
        Symbol(s) => eval_special_form_or_call_step(s, &list[1..], env, ctx, permitted),
        Form(_)   => /* valuta la testa, riscrivi la forma, TailCall */,
        Lambda(l) => /* chiama la chiusura direttamente */,
        _         => Err(UnvalidFunctionCall),
    },
    Vector(v) => /* valuta ogni elemento, Done(nuovo vettore) */,
    Map(m)    => /* valuta ogni valore, Done(nuova mappa) */,
}
```
]

#ref-table(
  columns: (auto, 1fr),
  header: ([Variante], [Valutazione]),
  [`String`, `Number`, `Atom`, `Fiber`, `Lambda`, `Primitive`], [Valgono sé stessi.],
  [`Symbol`], [`nil`, `t` e le keyword (`:nome`) valgono sé stessi, senza cercarli; altrimenti si cerca la variabile, oppure `UnboundVariable`.],
  [`Cons`], [Un dato valutato: si converte con `data_to_form` e si restituisce come `TailCall`, così la forma ottenuta passa di nuovo da qui.],
  [`Form` vuota], [`nil`.],
  [`Form` con testa simbolo], [Forma speciale, macro o chiamata di funzione.],
  [`Form` con testa `Form`], [La testa viene valutata e la forma riscritta con il valore al suo posto: `((lambda (x) (* x 2)) 21)` vale 42.],
  [`Form` con testa `Lambda`], [Chiamata diretta della chiusura: argomenti valutati, nuovo ambiente, frame `<lambda>` nella pila delle chiamate. Accade quando la riscrittura della riga precedente mette una chiusura in testa.],
  [`Form` con altra testa], [`UnvalidFunctionCall`: `(1 2 3)` non è una chiamata.],
  [`Vector`, `Map`], [Si valutano gli elementi o i valori; le chiavi di una mappa restano testo.],
)

== Le forme speciali

Quando la testa è un simbolo, `eval_special_form_or_call_step` confronta il nome
con le forme speciali, scritte direttamente nei rami del suo `match`. Gli
argomenti di una forma speciale arrivano #emph[non valutati]: è la forma a
decidere che cosa valutare, quando e in quale ambiente. La tabella è il
riferimento completo; la colonna «coda» dice quali sottoforme sono valutate
come chiamata in coda, la colonna `yield` se il corpo può sospendersi.

#ref-table(
  columns: (auto, 1fr, auto, auto),
  header: ([Forma], [Comportamento], [Coda], [`yield`]),
  [`quote`], [Restituisce l'argomento, già convertito in dato dal lettore.], [---], [---],
  [`backquote`], [Modello con `,` (un valore) e `,@` (gli elementi di una lista). Un solo livello.], [---], [no],
  [`if`], [`(if C ALLORA [ALTRIMENTI])`. Solo #emph[una] forma «altrimenti»: le successive sono ignorate.], [sì], [nei rami],
  [`cond`], [Clausole `(TEST CORPO...)`; senza corpo restituisce il valore del test.], [sì], [sì],
  [`and`, `or`], [Cortocircuito; `(and)` è `t`, `(or)` è `nil`.], [l'ultima], [l'ultima],
  [`when`, `unless`], [Corpo valutato come un `progn`.], [sì], [sì],
  [`progn`], [Valuta il corpo, restituisce l'ultima forma.], [sì], [sì],
  [`prog1`, `prog2`], [Restituiscono la prima o la seconda forma.], [no], [no],
  [`while`], [Ripete finché la condizione è vera. Restituisce il valore dell'ultima forma eseguita, `nil` se il corpo non gira mai.], [no], [sì],
  [`dolist`], [`(dolist (VAR LISTA [RISULTATO]) CORPO...)`.], [no], [no],
  [`dotimes`], [`(dotimes (VAR N [RISULTATO]) CORPO...)`, `VAR` da 0 a N−1.], [no], [no],
  [`setq`], [Coppie `NOME VALORE`; restituisce l'ultimo valore.], [no], [---],
  [`let`, `let*`], [Legami in parallelo o in sequenza; `(let ())` è ammesso, `(let)` no.], [sì], [sì],
  [`defvar`, `defconst`], [`(defvar NOME [VALORE [DOC]])`; restituisce il simbolo.], [no], [---],
  [`defun`], [`(defun NOME PARAMETRI [DOC] CORPO...)`.], [---], [---],
  [`lambda`], [Crea una chiusura sull'ambiente corrente; non registra docstring.], [---], [---],
  [`defmacro`], [Come `defun`, nello spazio delle macro; nessuna docstring.], [---], [---],
  [`catch`], [`(catch TAG CORPO...)`: intercetta i `throw` con lo stesso tag.], [no], [no],
  [`condition-case`], [`(condition-case VAR FORMA GESTORE...)`: intercetta gli errori.], [nel gestore], [no],
  [`unwind-protect`], [`(unwind-protect FORMA PULIZIA...)`: le pulizie girano sempre.], [no], [no],
  [`spawn`], [`(spawn (lambda () ...))`: esegue la chiusura su un nuovo thread.], [---], [---],
  [`fiber`], [`(fiber CORPO...)`: un programma sospendibile; `(fiber)` è già finito.], [---], [---],
  [`yield`], [`(yield [VALORE])`: sospende la fiber corrente.], [---], [---],
)

`throw`, `signal`, `resume`, `funcall`, `apply` ed `eval` non sono forme
speciali ma primitive: ricevono gli argomenti già valutati. Le sezioni seguenti
descrivono gli algoritmi delle forme raggruppate per funzione; `catch`,
`condition-case` e `unwind-protect` sono nel @errori, `fiber` e `yield` nel
@fiber, `spawn` nel @thread.

=== Corpi: `progn` e `eval_body_step`

Un #emph[corpo] è una sequenza di forme di cui conta solo il valore
dell'ultima. `progn`, `let`, `let*`, `when`, `unless`, le clausole di `cond` e
il corpo di ogni funzione lo valutano con la stessa funzione:

#estratto("eval.rs")[
```rust
fn eval_body_step<T>(body: &[LispExp<T>], env: Arc<Env<T>>, ctx: &T, permitted: bool)
    -> Result<EvalStep<T>, EvalError<T>>
{
    let Some((last, leading)) = body.split_last() else {
        return Ok(EvalStep::Done(LispExp::nil()));      // corpo vuoto: nil
    };
    run_statements(&body[..leading.len()], 0, &env, ctx, permitted, LispExp::nil(),
                   |from| Frame::Body { forms: body.to_vec(), from, env: env.clone() })?;
    Ok(EvalStep::TailCall(last.clone(), env))           // l'ultima: in coda
}
```
]

Le forme tranne l'ultima sono #emph[istruzioni]: il loro valore si scarta.
`run_statements` le valuta una a una, concedendo a ciascuna il permesso di
sospendersi se il corpo lo aveva, e sa descrivere «il resto del corpo» se una
di esse fa `yield` (il terzo argomento è la funzione che costruisce quella
descrizione, @fiber). L'ultima forma non viene valutata qui: torna al
trampolino come `TailCall`.

#canvas(height: 2.8cm)[
  #node(1.4cm, 0.5cm, 2.2cm, 0.6cm, [`(progn`], fill: paper)
  #node(3.9cm, 0.5cm, 2.2cm, 0.6cm, [`f1`])
  #node(6.4cm, 0.5cm, 2.2cm, 0.6cm, [`f2`])
  #node(8.9cm, 0.5cm, 2.2cm, 0.6cm, [`f3)`], fill: accent.lighten(85%))
  #group(2.6cm, 1.15cm, 5.1cm, 1.3cm, [`run_statements`])
  #note(2.9cm, 1.45cm)[`eval(f1)`, poi `eval(f2)`; valori scartati;\ un `yield` qui produce un `Frame::Body`]
  #carrow((8.9cm, 0.85cm), (11.2cm, 1.75cm), colour: accent)
  #node(13.4cm, 1.75cm, 4.2cm, 0.65cm, [`TailCall(f3, env)` al trampolino], fill: accent.lighten(85%))
]

=== Condizionali: `if`, `cond`, `and`, `or`, `when`, `unless`

Tutti valutano le condizioni con `eval` ricorsiva e restituiscono come
`TailCall` la forma da cui dipende il risultato:

- `if` valuta `C`; se è vera restituisce `TailCall(ALLORA)`, altrimenti
  `TailCall(ALTRIMENTI)` o, se manca, `nil`. Senza condizione è
  `IfNoConditionProvided`, senza ramo «allora» `IfNoTrueBrach`;
- `cond` prova le clausole in ordine; alla prima con il test vero valuta il
  corpo con `eval_body_step`, o restituisce il valore del test se il corpo è
  vuoto. Una clausola che non è una lista non vuota è `CondInvalidClause`;
- `and` valuta tutte le forme tranne l'ultima e si ferma con `nil` alla prima
  falsa; `or` si ferma con il primo valore vero; in entrambi i casi l'ultima
  forma è una chiamata in coda;
- `when` e `unless` valutano la condizione e poi il corpo con `eval_body_step`.

=== Cicli: `while`, `dolist`, `dotimes`

`while` valuta prima la condizione: se è falsa restituisce `nil` senza toccare
il corpo. Poi le strade sono due. Se la forma #emph[non] ha il permesso di
sospendersi, gira un ciclo semplice: valuta le forme del corpo con `eval`,
ricorda il valore dell'ultima, rivaluta la condizione. Se ha il permesso,
passa a `run_while`, la stessa funzione che userà la ripresa di una fiber
sospesa a metà di un'iterazione:

#estratto("eval.rs")[
```rust
fn run_while<T>(condition: &LispExp<T>, forms: &[LispExp<T>], from: usize,
                env: &Arc<Env<T>>, ctx: &T, carried: LispExp<T>)
    -> Result<LispExp<T>, EvalError<T>>
{
    let frame_at = |from| Frame::While { condition: …, forms: …, from, env: … };
    // finisce l'iterazione in corso (da FROM), poi itera normalmente
    let mut last = run_statements(forms, from, env, ctx, true, carried, frame_at)?;
    loop {
        if eval(condition, env.clone(), ctx)?.is_nil() { return Ok(last); }
        last = run_statements(forms, 0, env, ctx, true, LispExp::nil(), frame_at)?;
    }
}
```
]

Tutte le forme del corpo di un `while` sono istruzioni, e quindi tutte
possono sospendersi; la condizione no. `while` non crea un ambiente nuovo.

`dolist` e `dotimes` sono più semplici. Valutano la lista o il conteggio,
creano un ambiente figlio con la variabile del ciclo, e per ogni elemento (o
ogni intero da 0 a N−1) aggiornano la variabile con `update_variable` e valutano
le forme del corpo con `eval`. La forma `RISULTATO`, se c'è, è valutata alla
fine nell'ambiente del ciclo, dove la variabile vale ancora l'ultimo elemento
(per `dolist`) o N−1. Il conteggio di `dotimes` è troncato a intero; una lista
di `dolist` che non è una lista è `WrongArgumentType`. Il ciclo di questi due
costrutti è scritto in Rust e consuma carburante solo attraverso le forme del
corpo #rapporto("INT-5"); nessuna delle loro forme può sospendersi.

=== Legami: `setq`, `let`, `let*`, `defvar`, `defconst`

`setq` richiede un numero pari di argomenti (`SetqWrongNumberOfArgs`) e un
simbolo in ogni posizione dispari (`SetqSymbolRequired`). Per ogni coppia valuta
il valore e prova `update_variable`; se nessun anello ha la variabile, la crea
con `set_variable` nell'ambiente corrente. Le coppie sono assegnate in ordine,
quindi `(setq a 1 b a)` dà a `b` il nuovo valore di `a`.

`let` crea un figlio, valuta #emph[ogni] valore nell'ambiente #emph[esterno] e
lo lega nel figlio; poi valuta il corpo nel figlio. `let*` è identico, ma
valuta ogni valore nel figlio stesso, dove i legami precedenti sono già
visibili. Un legame è `(NOME FORMA)` oppure un `NOME` nudo, che vale `nil`
(`parse_let_binding`); ogni altra forma è `LetUnvalidBindingAt(indice)`.

```lisp
(let ((x 1)) (let  ((x 2) (y x)) y))   ; => 1: y vede la x esterna
(let ((x 1)) (let* ((x 2) (y x)) y))   ; => 2: y vede la x appena legata
```

`defvar` e `defconst` controllano la forma prima di toccare qualunque cosa (al
massimo tre argomenti, il primo un simbolo, la docstring una stringa), poi
mettono la docstring nelle proprietà del simbolo #emph[a ogni valutazione], e
infine assegnano: `defconst` sempre, `defvar` solo se `get_variable` non trova
la variabile. È questa regola che permette di ricaricare un modulo senza
perdere un valore che l'utente ha cambiato nel frattempo. Il valore è legato
nell'ambiente corrente.

=== Definizioni: `defun`, `lambda`, `defmacro`

Le tre forme costruiscono una `Lambda` e differiscono solo in ciò che ne fanno:
`lambda` la restituisce, `defun` la lega nello spazio delle funzioni,
`defmacro` in quello delle macro. In tutti e tre i casi l'ambiente catturato è
quello corrente.

La lista dei parametri è analizzata da `parse_lambda_params`, un piccolo
automa a quattro stati che smista i simboli in tre gruppi:

#canvas(height: 2.7cm)[
  #node(1.8cm, 1.0cm, 2.6cm, 0.7cm, [`Required`])
  #node(6.0cm, 1.0cm, 2.6cm, 0.7cm, [`Optional`])
  #node(10.2cm, 1.0cm, 2.6cm, 0.7cm, [`Rest`])
  #node(14.4cm, 1.0cm, 2.6cm, 0.7cm, [`RestDone`])
  #carrow((3.1cm, 1.0cm), (4.7cm, 1.0cm), label: [`&optional`], dy: -8pt)
  #carrow((7.3cm, 1.0cm), (8.9cm, 1.0cm), label: [`&rest`], dy: -8pt)
  #carrow((11.5cm, 1.0cm), (13.1cm, 1.0cm), label: [un nome], dy: -8pt)
  #segment((1.8cm, 1.35cm), (1.8cm, 1.85cm))
  #segment((1.8cm, 1.85cm), (10.2cm, 1.85cm))
  #carrow((10.2cm, 1.85cm), (10.2cm, 1.35cm))
  #note(4.2cm, 1.9cm)[`&rest` anche direttamente dagli obbligatori]
  #note(0.3cm, 0.1cm)[un nome → obbligatori]
  #note(4.9cm, 0.1cm)[un nome → facoltativi]
]

Un `&optional` fuori da `Required`, o un secondo `&rest`, è
`DefunMisplacedParamMarker`; un `&rest` senza nome dopo, o con più di un nome,
è `DefunRestMustHaveExactlyOneParam`; un parametro che non è un simbolo è
`DefunParamIsNotASymbol`.

`defun` considera docstring il terzo argomento solo se è una stringa #emph[e]
dopo di esso ci sono altre forme: `(defun f () "ciao")` è una funzione che
restituisce `"ciao"`. `lambda` e `defmacro` non registrano docstring.

== Chiamate di funzione e macro <chiamate>

Se il nome in testa non è una forma speciale, `eval_macro_or_function_call_step`
cerca prima una macro e poi una funzione.

#canvas(height: 6.4cm)[
  #node(8.1cm, 0.5cm, 5.4cm, 0.8cm, [`(nome arg1 arg2 ...)`], fill: paper)
  #carrow((8.1cm, 0.9cm), (8.1cm, 1.45cm))
  #node(8.1cm, 1.85cm, 5.4cm, 0.8cm, [`nome` è una forma speciale?])
  #carrow((10.8cm, 1.85cm), (11.85cm, 1.85cm), label: [sì], dy: -7pt)
  #node(14.0cm, 1.85cm, 4.2cm, 0.8cm, [ramo del `match`\ (argomenti non valutati)])
  #arrow((8.1cm, 2.25cm), (8.1cm, 2.85cm), label: [no], label-dx: 5pt, label-dy: -5pt)
  #node(8.1cm, 3.25cm, 5.4cm, 0.8cm, [`env.get_macro(nome)`?])
  #carrow((10.8cm, 3.25cm), (11.85cm, 3.25cm), label: [sì], dy: -7pt)
  #node(14.0cm, 3.25cm, 4.2cm, 1.0cm, [lega gli argomenti #emph[non valutati],\ valuta il corpo → `TailCall(espansione)`])
  #arrow((8.1cm, 3.65cm), (8.1cm, 4.25cm), label: [no], label-dx: 5pt, label-dy: -5pt)
  #node(8.1cm, 4.65cm, 5.4cm, 0.8cm, [valuta gli argomenti, `env.get_function(nome)`])
  #carrow((6.4cm, 5.05cm), (3.6cm, 5.75cm))
  #carrow((9.8cm, 5.05cm), (12.6cm, 5.75cm))
  #node(3.6cm, 6.05cm, 5.6cm, 0.6cm, [`Lambda`: nuovo `Env`, `bind_lambda_args`, `eval_body_step`], size: 8pt)
  #node(12.6cm, 6.05cm, 5.6cm, 0.6cm, [`Primitive`: `pointer(args, env, ctx)` → `Done`], size: 8pt)
  #note(0.2cm, 4.3cm)[nessuna delle due:\ `UndefinedFunction`]
]

=== Chiamare una funzione

#estratto("eval.rs")[
```rust
fn eval_function_call_step<T>(symbol: &str, args: &[LispExp<T>], env: Arc<Env<T>>,
                              ctx: &T, permitted: bool) -> Result<EvalStep<T>, EvalError<T>>
{
    let mut evaled_args = Vec::new();
    for arg in args { evaled_args.push(eval(arg, env.clone(), ctx)?); }  // da sinistra

    if let Some(func) = env.get_function(symbol) {
        ctx.push_call_frame(symbol);
        if let LispExp::Lambda(lambda) = func {
            let call_frame = Env::new_child(&lambda.env);     // figlio della DEFINIZIONE
            bind_lambda_args(&lambda, &evaled_args, &call_frame)?;
            let step = eval_body_step(&lambda.body, call_frame, ctx, permitted);
            pop_unless_failed(ctx, &step);
            step
        } else if let LispExp::Primitive { pointer, .. } = func {
            let result = pointer(&evaled_args[..], env.clone(), ctx)?; // env del CHIAMANTE
            ctx.pop_call_frame();
            Ok(EvalStep::Done(result))
        } else { Err(EvalError::UncorrectFunctionDefinition) }
    } else { Err(EvalError::UndefinedFunction(symbol.into())) }
}
```
]

L'ordine conta. Gli argomenti sono valutati #emph[prima] di cercare la
funzione, ciascuno con una `eval` ricorsiva, quindi senza permesso di
sospendersi. Il frame della pila delle chiamate è aperto prima di legare gli
argomenti, così un numero sbagliato di argomenti compare nel backtrace con il
nome della funzione chiamata. Una `Lambda` riceve un ambiente figlio del
#emph[suo] ambiente; una primitiva riceve l'ambiente del #emph[chiamante], ed è
per questo che `eval`, `eval-string` e `boundp` vedono le variabili locali di
chi le chiama. Il corpo della funzione non viene valutato qui: `eval_body_step`
valuta le istruzioni e restituisce l'ultima forma come `TailCall`, e il frame
viene tolto #emph[prima] che il trampolino la valuti.

=== Legare gli argomenti: `bind_lambda_args`

#estratto("utils.rs")[
```rust
let min = lambda.params.len();
let max = min + lambda.optionals.len();
if args.len() < min || (lambda.rest.is_none() && args.len() > max) {
    return Err(WrongNumberOfArguments { expected: if args.len() < min { min } else { max },
                                        got: args.len() });
}
let mut idx = 0;
for name in &lambda.params {
    call_frame.set_variable(name.clone(), args[idx].clone());
    idx += 1;
}
for name in &lambda.optionals {
    let value = args.get(idx).cloned().unwrap_or_else(LispExp::nil);
    call_frame.set_variable(name.clone(), value);
    idx += 1;
}
if let Some(rest) = &lambda.rest {
    let start = idx.min(args.len());
    call_frame.set_variable(rest.clone(), LispExp::proper_list(args[start..].to_vec()));
}
```
]

#ref-table(
  columns: (4.4cm, 2.0cm, 1fr),
  header: ([Parametri], [Argomenti], [Legami]),
  [`(a b)`], [`1 2`], [`a=1`, `b=2`],
  [`(a b)`], [`1`], [`WrongNumberOfArguments { expected: 2, got: 1 }`],
  [`(a &optional b c)`], [`1 2`], [`a=1`, `b=2`, `c=nil`],
  [`(a &optional b)`], [`1 2 3`], [`WrongNumberOfArguments { expected: 2, got: 3 }`],
  [`(a &rest r)`], [`1 2 3`], [`a=1`, `r=(2 3)`],
  [`(a &optional b &rest r)`], [`1`], [`a=1`, `b=nil`, `r=nil` (l'indice è limitato con `min`)],
  [`(&rest r)`], [nessuno], [`r=nil`],
)

La stessa funzione lega gli argomenti di una macro. Lì gli «argomenti» sono le
sottoforme non valutate, e il contenitore di `&rest` è comunque una lista di
dati, che il corpo della macro scorre con `car` e `cdr`.

=== Espandere una macro

#estratto("eval.rs")[
```rust
if let Some(LispExp::Lambda(macro_lambda)) = env.get_macro(symbol) {
    let expand_frame = Env::new_child(&macro_lambda.env);
    bind_lambda_args(&macro_lambda, args, &expand_frame)?;   // argomenti NON valutati
    let mut expansion = LispExp::nil();
    for form in &macro_lambda.body {
        expansion = eval(form, expand_frame.clone(), ctx)?;   // il corpo produce un dato
    }
    Ok(EvalStep::TailCall(expansion, env))                   // valutata dove sta la chiamata
}
```
]

Una macro è una funzione che riceve codice e restituisce codice. Il percorso di
`(my-when t 1 2)` con la macro dell'esempio:

```lisp
(defmacro my-when (c &rest body) `(if ,c (progn ,@body)))
(my-when t 1 2)
```

+ `get_macro("my-when")` trova la `Lambda`; si crea un figlio del suo ambiente.
+ `bind_lambda_args` lega `c` al simbolo `t` e `body` alla lista `(1 2)`: sono
  le sottoforme come le ha scritte il chiamante.
+ Il corpo, un backquote, produce il #emph[dato] `(if t (progn 1 2))`, una catena
  di `Cons`.
+ Il dato torna al trampolino come `TailCall` nell'ambiente del #emph[chiamante].
+ Al giro successivo `eval_step_permitted` vede una `Cons`, la converte con
  `data_to_form` in una `Form` e la restituisce di nuovo come `TailCall`.
+ Il giro dopo valuta `(if t (progn 1 2))` come qualunque altra forma: vale `2`.

L'espansione avviene a ogni valutazione: non c'è una cache. Poiché è una
chiamata in coda, eredita il permesso di sospendersi della posizione in cui
compare la chiamata alla macro. Non esiste `gensym`: una macro che introduce
variabili temporanee usa nomi che il codice dell'utente non userà
(`save-match-data--saved` in `core/lisp/commands.lisp` ne è un esempio).

=== Il backquote

`eval_backquote` percorre il modello e costruisce un dato:

#estratto("eval.rs")[
```rust
match exp {
    LispExp::Form(list) => {
        if is_tagged(list, "unquote") { return eval(&list[1], env, ctx); }   // ,X
        let mut result = Vec::new();
        for item in list.iter() {
            if item is Form(inner) && is_tagged(inner, "unquote-splicing") {  // ,@X
                let spliced = eval(&inner[1], env.clone(), ctx)?;
                // Cons o Form: gli elementi; nil: niente; altro: errore
                result.extend(elements of spliced);
                continue;
            }
            result.push(eval_backquote(item, env.clone(), ctx)?);            // ricorsione
        }
        Ok(LispExp::proper_list(result))
    }
    LispExp::Vector(vec) => /* ogni elemento con eval_backquote, poi un vettore */,
    _ => Ok(exp.clone()),             // atomi, Cons, mappe: invariati
}
```
]

#canvas(height: 3.4cm)[
  #node(5.2cm, 0.5cm, 9.6cm, 0.65cm, [modello #raw("`(a ,x ,@ys (b ,x))") con `x = 1` e `ys = (2 3)`], fill: paper)
  #node(1.1cm, 1.9cm, 1.4cm, 0.6cm, [`a`])
  #node(3.3cm, 1.9cm, 2.0cm, 0.6cm, [`,x` → `1`])
  #node(6.0cm, 1.9cm, 2.6cm, 0.6cm, [`,@ys` → `2`, `3`])
  #node(9.6cm, 1.9cm, 3.6cm, 0.6cm, [`(b ,x)` → ricorsione → `(b 1)`])
  #carrow((2.0cm, 0.85cm), (1.2cm, 1.6cm))
  #carrow((3.6cm, 0.85cm), (3.3cm, 1.6cm))
  #carrow((5.4cm, 0.85cm), (6.0cm, 1.6cm))
  #carrow((7.6cm, 0.85cm), (9.4cm, 1.6cm))
  #carrow((11.45cm, 1.9cm), (12.2cm, 1.9cm))
  #node(14.1cm, 1.9cm, 3.6cm, 0.65cm, [`(a 1 2 3 (b 1))`], fill: paper)
  #note(0.2cm, 2.6cm)[il risultato è una lista di dati costruita con `proper_list`; `,@` aggiunge gli #emph[elementi] della lista, `,` un solo valore]
]

Un atomo, una `Cons` e una mappa sono restituiti come sono. Una lista puntata
scritta nel modello è già una `Cons`, e quindi le sue virgole non vengono
espanse; lo stesso vale per i valori di una mappa e per un backquote annidato
#rapporto("INT-11").

=== Chiamare da Rust: `call_callable`

#estratto("base/mod.rs")[
```rust
pub fn call_callable<T>(func: &LispExp<T>, call_args: &[LispExp<T>],
                        env: Arc<Env<T>>, ctx: &T) -> Result<LispExp<T>, EvalError<T>>
{
    match func {
        LispExp::Lambda(lambda) => {
            let call_frame = Env::new_child(&lambda.env);
            bind_lambda_args(lambda, call_args, &call_frame)?;
            // ogni forma con eval diretta; l'ultima dà il valore
            for exp in &lambda.body[..last] { eval(exp, call_frame.clone(), ctx)?; }
            eval(lambda.body.last(), call_frame, ctx)
        }
        LispExp::Primitive { pointer: f, .. } => f(call_args, env, ctx),
        LispExp::Symbol(name) => match env.get_function(name) {
            Some(resolved) => call_callable(&resolved, call_args, env, ctx),
            None => Err(EvalError::UndefinedFunction(name.to_string())),
        },
        _ => Err(EvalError::UncorrectFunctionDefinition),
    }
}
```
]

È il modo di chiamare qualcosa che si ha già in mano --- una lambda, una
primitiva o un simbolo che nomina una funzione --- con argomenti già valutati.
La usano `funcall`, `apply`, `mapcar`, `mapc` e l'editor quando richiama Lisp da
Rust (hook, callback del minibuffer, lavori di sfondo). Rispetto a una chiamata
del valutatore ha tre differenze: non registra un frame nel backtrace, non
concede il permesso di sospendersi (un `yield` nel corpo è `YieldNotAllowed`),
e l'ultima forma è valutata con una `eval` ricorsiva invece che in coda.

== La pila delle chiamate per i backtrace <backtrace>

Prima del corpo di una funzione o di una primitiva il valutatore chiama
`ctx.push_call_frame(nome)` (`<lambda>` per le chiusure chiamate in testa a
una forma). La pila appartiene all'host; il valutatore si limita a dire quando
un frame entra e quando esce, con un protocollo preciso implementato da
`pop_unless_failed`:

#ref-table(
  columns: (auto, 1fr),
  header: ([Come finisce la chiamata], [Cosa succede al frame]),
  [valore o chiamata in coda], [`pop`: il lavoro del frame è finito.],
  [errore], [resta: quando l'errore arriva in cima, la pila descrive esattamente le chiamate attive al momento del guasto, ed è ciò che l'editor mostra come backtrace.],
  [sospensione (`yield`)], [`pop`: una fiber parcheggiata non sta chiamando nessuno.],
)

#canvas(height: 3.4cm)[
  #segment((0.4cm, 2.9cm), (16.2cm, 2.9cm), colour: dim)
  #note(15.4cm, 3.0cm)[tempo]
  #node(1.9cm, 0.5cm, 2.6cm, 0.6cm, [`push("f")`])
  #node(4.9cm, 0.5cm, 2.6cm, 0.6cm, [`push("g")`])
  #node(7.9cm, 0.5cm, 2.6cm, 0.6cm, [`push("car")`])
  #node(11.1cm, 0.5cm, 3.2cm, 0.6cm, [`car` fallisce], fill: warm.lighten(85%))
  #node(14.6cm, 0.5cm, 3.0cm, 0.6cm, [pila: `f g car`], fill: paper)
  #note(0.4cm, 1.3cm)[`(defun f () (g) nil)` · `(defun g () (car 5) nil)` · `(f)`]
  #note(0.4cm, 1.8cm)[Nessun frame viene tolto durante lo srotolamento: l'host trova in cima esattamente le tre chiamate attive.\ Se lo stesso errore fosse intercettato, chi lo intercetta riporterebbe la pila alla profondità che aveva annotato.]
]

Chi intercetta un errore (`catch`, `condition-case`, `unwind-protect`,
`eval-string-safe`) annota `call_frame_depth()` prima e chiama
`truncate_call_frames` dopo, così i frame di un fallimento già gestito non
compaiono nel backtrace successivo.

Le chiamate in coda non appaiono nella pila: il frame di una funzione viene
tolto quando il suo corpo restituisce l'ultima forma al trampolino, cioè
#emph[prima] che quella forma venga valutata. Nell'esempio le due funzioni
terminano con `nil` proprio perché la chiamata che fallisce non sia l'ultima
forma: con `(defun g () (car 5))` il frame di `g` sarebbe già uscito quando
`car` fallisce, e la pila conterrebbe solo `f` e `car`.

// ===========================================================================
= Errori e uscite non locali <errori>
// ===========================================================================

== Un solo canale per tre cose diverse

`EvalError<T>` è generico su `T` perché alcune varianti portano valori Lisp.
Le varianti si dividono in tre famiglie.

#ref-table(
  columns: (auto, 1fr, auto),
  header: ([Famiglia], [Varianti], [Chi le ferma]),
  [Errori], [`UnboundVariable`, `UndefinedFunction`, `WrongNumberOfArguments`, `WrongArgumentType`, `OutOfFuel`, `RuntimeMessage`, `Signal { symbol, data }`, `YieldNotAllowed` e le forme malformate (tabella sotto)], [`condition-case`],
  [Trasferimenti], [`Throw { tag, value }`], [il `catch` con lo stesso tag],
  [Sospensioni], [`Yielded { value, frames }`], [`resume`],
)

Usare `Err` anche per `throw` e `yield` è la scelta naturale in Rust: `?`
propaga già un `Err` attraverso ogni frame, quindi nessun punto del valutatore
deve sapere che un trasferimento lo sta attraversando. Chi deve fermarne uno lo
riconosce per variante.

#ref-table(
  columns: (7.0cm, 1fr),
  header: ([Errore di forma malformata], [Esempio che lo produce]),
  [`QuoteNotOneArgument`], [`(quote a b)`],
  [`IfNoConditionProvided`, `IfNoTrueBrach`], [`(if)`, `(if t)`],
  [`SetqSymbolRequired`, `SetqWrongNumberOfArgs`], [`(setq 1 2)`, `(setq a)`],
  [`DefunNameMustBeASymbol`, `DefunNotCorrectExpression`], [`(defun "f" () 1)`, `(defun f ())`],
  [`DefunParamsAreNotAList`, `DefunParamIsNotASymbol`], [`(defun f x 1)`, `(lambda (1) 1)`],
  [`DefunMisplacedParamMarker`, `DefunRestMustHaveExactlyOneParam`], [`(lambda (&rest a &optional b) 1)`, `(lambda (&rest) 1)`],
  [`LetNoBindingsProvided`, `LetUnvalidBindingList`, `LetUnvalidBindingAt(i)`], [`(let)`, `(let 5 1)`, `(let ((1 2)) 1)`],
  [`CondInvalidClause`], [`(cond 5)`, `(cond ())`],
  [`DolistInvalidBinding`, `DotimesInvalidBinding`], [`(dolist x)`, `(dotimes (i))`],
  [`DefvarNameMustBeASymbol`, `DefvarDocMustBeAString`], [`(defvar)`, `(defvar x 1 2)`],
  [`BackquoteNotOneArgument`], [`(backquote a b)`],
  [`ConditionCaseInvalidVariable`, `ConditionCaseInvalidHandler`], [`(condition-case 5 x)`, `(condition-case e x 5)`],
  [`UnvalidFunctionCall`, `UncorrectFunctionDefinition`], [`(1 2)`, una lista puntata valutata; un valore non chiamabile nello spazio delle funzioni],
)

== Come si propaga

#canvas(height: 5.0cm)[
  #node(2.8cm, 0.5cm, 5.2cm, 0.8cm, [`(catch 'fine ...)`], fill: paper)
  #node(2.8cm, 1.75cm, 5.2cm, 0.8cm, [`(condition-case e ...)`], fill: paper)
  #node(2.8cm, 3.0cm, 5.2cm, 0.8cm, [`(unwind-protect corpo pulizia ...)`], fill: paper)
  #node(2.8cm, 4.25cm, 5.2cm, 0.8cm, [primitiva o forma che fallisce], fill: warm.lighten(85%))
  #carrow((5.8cm, 4.25cm), (5.8cm, 3.0cm), colour: warm)
  #carrow((5.8cm, 3.0cm), (5.8cm, 1.75cm), colour: warm)
  #carrow((5.8cm, 1.75cm), (5.8cm, 0.5cm), colour: warm)

  #note(6.3cm, 0.25cm)[ferma soltanto `Throw` con il proprio tag;\ ogni altro errore prosegue verso l'alto]
  #note(6.3cm, 1.5cm)[ferma gli errori la cui condizione corrisponde a un gestore;\ lascia passare `Throw` intatto]
  #note(6.3cm, 2.75cm)[esegue #emph[tutte] le forme di pulizia (con il carburante concesso\ da `begin_unwind`), poi lascia proseguire l'errore del corpo]
  #note(6.3cm, 4.0cm)[restituisce `Err(...)`, che risale con `?` frame dopo frame]
]

Un errore che nessuno ferma arriva a chi ha chiamato `eval` per primo: nel
caso dell'editor, `run_command_form`, che lo mostra nell'area dei messaggi
insieme al backtrace.

== `catch` e `throw`

`throw` è una primitiva che restituisce `Err(Throw { tag, value })`. `catch`
valuta il tag, annota la profondità della pila delle chiamate, poi valuta le
forme del corpo una a una #emph[dentro il proprio frame Rust]:

#estratto("eval.rs")[
```rust
let tag = eval(&args[0], env.clone(), ctx)?;
let depth_before = ctx.call_frame_depth();
let mut result = LispExp::nil();
for form in &args[1..] {
    match eval(form, env.clone(), ctx) {
        Ok(value) => result = value,
        Err(EvalError::Throw { tag: thrown, value }) => {
            if thrown == tag {                          // PartialEq: equal
                ctx.truncate_call_frames(depth_before);
                return Ok(EvalStep::Done(value));
            }
            return Err(EvalError::Throw { tag: thrown, value });   // non è il mio
        }
        Err(err) => return Err(err),
    }
}
Ok(EvalStep::Done(result))
```
]

Le forme non possono essere restituite al trampolino come `TailCall`: il
trampolino le valuterebbe #emph[fuori] dal frame di `catch`, e il `throw`
passerebbe oltre. Il tag si confronta con `PartialEq`, cioè strutturalmente:
due stringhe uguali sono lo stesso tag. Un `throw` senza `catch` corrispondente
arriva in cima come errore.

== `condition-case`

#estratto("eval.rs")[
```rust
let depth_before = ctx.call_frame_depth();
let err = match eval(&args[1], env.clone(), ctx) {
    Ok(value) => return Ok(EvalStep::Done(value)),
    Err(throw @ EvalError::Throw { .. }) => return Err(throw),   // non è un errore
    Err(err) => err,
};
let symbol = error_symbol(&err);
for handler in &args[2..] {
    let clause = /* (CONDIZIONE CORPO...) non vuota, o ConditionCaseInvalidHandler */;
    if !condition_matches(&clause[0], &symbol) { continue; }
    ctx.truncate_call_frames(depth_before);
    let handler_env = Env::new_child(&env);
    if let Some(name) = &var {
        let bound = LispExp::cons(symbol.clone(), error_data(&err));
        handler_env.set_variable(name.clone(), bound);
    }
    if clause.len() == 1 { return Ok(EvalStep::Done(LispExp::nil())); }
    for form in &clause[1..clause.len() - 1] { eval(form, handler_env.clone(), ctx)?; }
    return Ok(EvalStep::TailCall(clause[clause.len() - 1].clone(), handler_env));
}
Err(err)    // nessun gestore: l'errore prosegue
```
]

La forma protetta è valutata nel frame di `condition-case`, per lo stesso
motivo di `catch`. Un `Throw` passa intatto: è un trasferimento di controllo,
non un fallimento. Per un errore vero, il nome della condizione viene da
`error_symbol` e i gestori sono provati in ordine. Il primo che corrisponde
riceve un ambiente figlio in cui `VAR` (se non è `nil`) vale
`(SIMBOLO . DATI)`; il suo corpo è valutato come un corpo, con l'ultima forma in
coda, perché a quel punto non c'è più nulla da proteggere.

#ref-table(
  columns: (1.3fr, 1fr, 1fr),
  header: ([`EvalError`], [Condizione (`error_symbol`)], [Dati (`error_data`)]),
  [`Signal { symbol, data }`], [il simbolo stesso], [`data`],
  [`UnboundVariable(n)`], [`unbound-variable`], [`(n)`],
  [`UndefinedFunction(n)`], [`undefined-function`], [`(n)`],
  [`UnvalidFunctionCall`, `UncorrectFunctionDefinition`], [`invalid-function`], [la descrizione dell'errore],
  [`WrongNumberOfArguments`], [`wrong-number-of-arguments`], [`(atteso ricevuto)`],
  [`WrongArgumentType`], [`wrong-type-argument`], [`("Tipo" valore)`],
  [`OutOfFuel`], [`out-of-fuel`], [la descrizione dell'errore],
  [`RuntimeMessage(m)`], [`runtime-error`], [`(m)`],
  [tutte le altre, compreso `YieldNotAllowed`], [`error`], [la descrizione dell'errore],
)

`condition_matches` decide se una condizione copre il simbolo: `t` ed `error`
coprono tutto, un simbolo copre sé stesso, una lista di condizioni copre ciò che
copre uno qualunque dei suoi elementi. Non esistono gerarchie di condizioni
(`define-error`): un gestore `error` intercetta anche i segnali con un simbolo
inventato.

```lisp
(condition-case e (car 5) (wrong-type-argument e))
;; => (wrong-type-argument "List" 5)
(condition-case nil (signal 'mio-errore '(1 2)) ((foo mio-errore) 'preso))
;; => preso
```

Intercettare `out-of-fuel` non serve a recuperare: quando l'errore arriva al
gestore il carburante del thread è a zero, e il primo passo del gestore fallisce
di nuovo con `OutOfFuel`. Il carburante si ricarica solo all'apertura del
prossimo scope esterno (@carburante).

== `unwind-protect`

#estratto("eval.rs")[
```rust
let body_result = eval(&args[0], env.clone(), ctx);
let depth_after_body = ctx.call_frame_depth();
ctx.begin_unwind();                         // carburante per le pulizie
let mut cleanup_error = None;
for cleanup in &args[1..] {
    if let Err(err) = eval(cleanup, env.clone(), ctx) {
        if cleanup_error.is_none() { cleanup_error = Some(err); }   // tiene il primo
    }
}
ctx.truncate_call_frames(depth_after_body);
match (body_result, cleanup_error) {
    (Err(body), Some(cleanup)) => { ctx.log_diagnostic(/* anche la pulizia */); Err(body) }
    (Err(body), None)          => Err(body),
    (Ok(_), Some(cleanup))     => Err(cleanup),
    (Ok(value), None)          => Ok(EvalStep::Done(value)),
}
```
]

Le pulizie girano #emph[tutte], anche se una di esse fallisce, e anche quando il
corpo è uscito con un `throw` o con `OutOfFuel`. La tabella riassume l'esito:

#ref-table(
  columns: (1fr, 1fr, 1.4fr),
  header: ([Corpo], [Pulizie], [Risultato]),
  [valore], [tutte riuscite], [il valore del corpo],
  [valore], [una fallita], [l'errore della prima pulizia fallita],
  [errore o `throw`], [tutte riuscite], [l'errore del corpo, che prosegue],
  [errore o `throw`], [una fallita], [l'errore del corpo; quello della pulizia va nella diagnostica],
)

Prima delle pulizie il valutatore chiama `ctx.begin_unwind()`. Nell'editor
questo alza il carburante rimasto ad almeno 100 000 passi, così un corpo uscito
per `OutOfFuel` può ancora eseguire le sue pulizie; la chiamata avviene dopo
ogni corpo, anche riuscito #rapporto("INT-1").

== `signal`, `eval-string-safe` e gli errori dell'host

`signal` è una primitiva che restituisce `Err(Signal { symbol, data })`: è il
modo con cui il codice Lisp, e le primitive dell'host, sollevano un errore con
un nome proprio. Una primitiva dell'editor che vuole un errore intercettabile
come `invalid-regexp` restituisce proprio questa variante.

`eval-string-safe` è una barriera: legge e valuta la prima espressione della
stringa, ripulisce la pila delle chiamate e restituisce sempre una lista,
`(t VALORE)` o `(nil "descrizione dell'errore")`. Un errore del lettore diventa
un `RuntimeMessage` con la descrizione del `ParserError`. È ciò che usa `M-:`
nell'editor.

// ===========================================================================
= Fiber e `yield` <fiber>
// ===========================================================================

Una fiber è un programma che può fermarsi a metà, consegnare un valore a chi
l'ha avviato e ripartire più tardi dallo stesso punto, con variabili e cicli
come li aveva lasciati. È il meccanismo su cui l'editor costruisce i lavori di
sfondo: un worker è una fiber che lo scheduler riprende un turno alla volta.

```lisp
(setq f (fiber (let ((i 0))
                 (while (< i 2)
                   (setq i (+ i 1))
                   (yield i))
                 'fine)))
(resume f)   ; => 1
(resume f)   ; => 2
(resume f)   ; => fine   e ora (fiber-done-p f) è t
(resume f)   ; => nil
```

== Il problema: lo stack di Rust non si può conservare

Il valutatore è una funzione Rust ricorsiva: mentre valuta il `yield` dentro il
`while` dentro il `let`, il punto in cui il programma si trova #emph[è] lo stack
di Rust, con un frame per ciascuna di quelle forme. Per consegnare il valore a
chi ha chiamato `resume` bisogna tornare a quel chiamante, cioè srotolare lo
stack, e srotolarlo lo distrugge.

La soluzione è che, durante lo srotolamento, ogni blocco attraversato scriva
#emph[che cosa gli resta da fare] in una struttura dati, un `Frame`. Alla
ripresa non si ricostruisce lo stack: si esegue quella descrizione. Questo
funziona solo nei punti in cui «ciò che resta da fare» è descrivibile
semplicemente, e il meccanismo dei permessi serve a garantire che un `yield`
avvenga solo lì.

== La rappresentazione

#estratto("types.rs")[
```rust
pub enum Frame<T: LispContext> {
    // il resto di un corpo: le forme da FROM in poi, nell'ambiente ENV
    Body  { forms: Vec<LispExp<T>>, from: usize, env: Arc<Env<T>> },
    // il resto di un'iterazione di un while, poi il ciclo stesso
    While { condition: LispExp<T>, forms: Vec<LispExp<T>>, from: usize, env: Arc<Env<T>> },
}
pub struct FiberState<T: LispContext> {
    pub pending: Vec<Frame<T>>,   // il blocco più interno per primo
    pub is_done: bool,
}
pub struct SharedFiber<T: LispContext>(pub Arc<RwLock<FiberState<T>>>);
```
]

Una fiber appena creata da `(fiber CORPO...)` è una pila con un solo
`Frame::Body` che contiene tutto il corpo, da `from = 0`, in un ambiente figlio
di quello in cui la fiber è stata creata: non esiste un percorso separato per
«la prima esecuzione», che è semplicemente una ripresa da prima della prima
forma. `(fiber)` senza corpo nasce già finita.

== Dove si può sospendere

Un'istruzione di un corpo e un'istruzione del corpo di un `while` sono i soli
punti in cui si sa descrivere il seguito: le istruzioni successive, e nel caso
del `while` il ciclo stesso. Il permesso di sospendersi è una variabile del
thread, `YIELD_PERMITTED`:

- lo #emph[concede] `run_statements`, prima di ogni istruzione, se il blocco
  stesso lo aveva; lo concede in ogni caso quando esegue un frame durante una
  ripresa;
- lo #emph[consuma] `eval_step` all'inizio di ogni passo, così qualunque
  valutazione annidata parte senza;
- lo #emph[restituisce] `eval_step` quando il passo finisce con una chiamata in
  coda, perché la forma che continua è la stessa istruzione.

Il `yield` controlla il permesso #emph[prima] di valutare il proprio argomento:

#estratto("eval.rs")[
```rust
"yield" => {
    if !permitted { return Err(EvalError::YieldNotAllowed); }
    let value = match args.first() {
        Some(form) => eval(form, env, ctx)?,   // un argomento: niente permesso
        None => LispExp::nil(),
    };
    Err(EvalError::Yielded { value, frames: Vec::new() })
}
```
]

Ne discende la tabella delle posizioni, verificata eseguendo l'interprete:

#ref-table(
  columns: (1fr, 1fr),
  header: ([Posizione del `yield`], [Esito]),
  [istruzione di `progn`, `let`, `let*`, `when`, `unless`, `cond`, corpo di funzione chiamata per nome], [sospende],
  [istruzione del corpo di `while` (quando il `while` stesso è in una posizione permessa)], [sospende],
  [ramo di `if`, ultima forma di `and`/`or`, espansione di una macro (chiamate in coda)], [sospende],
  [argomento di una chiamata, condizione, valore di un `let`], [`YieldNotAllowed`],
  [`dolist`, `dotimes`, `prog1`, `prog2`], [`YieldNotAllowed`],
  [`catch`, `unwind-protect`, `condition-case`], [`YieldNotAllowed`, che è un errore come gli altri: un gestore `error` di `condition-case` lo intercetta #rapporto("INT-10")],
  [chiusura chiamata con `funcall`, `mapc`, `mapcar` (`call_callable`)], [`YieldNotAllowed`],
  [fuori da una fiber], [`YieldNotAllowed`],
)

== Lo srotolamento: chi scrive i frame

`run_statements` è il cuore del meccanismo: valuta le istruzioni e, se una di
esse restituisce `Yielded`, aggiunge alla lista dei frame la descrizione del
proprio seguito e lascia proseguire l'errore.

#estratto("eval.rs")[
```rust
fn run_statements<T, F>(forms: &[LispExp<T>], from: usize, env: &Arc<Env<T>>, ctx: &T,
                        permitted: bool, carried: LispExp<T>, frame_at: F)
    -> Result<LispExp<T>, EvalError<T>>
where F: Fn(usize) -> Frame<T>
{
    let mut last = carried;
    for (index, form) in forms.iter().enumerate().skip(from) {
        if permitted { grant_yield_permission(); }
        match eval(form, env.clone(), ctx) {
            Ok(value) => last = value,
            Err(EvalError::Yielded { value, mut frames }) => {
                frames.push(frame_at(index + 1));        // riprendi dalla successiva
                return Err(EvalError::Yielded { value, frames });
            }
            Err(other) => return Err(other),
        }
    }
    Ok(last)
}
```
]

Poiché ogni blocco aggiunge il proprio frame #emph[dopo] quelli dei blocchi più
interni, la lista finisce ordinata dal più interno al più esterno. Il frame
punta all'istruzione #emph[successiva] (`index + 1`): quella che ha fatto
`yield` è considerata finita, e il suo valore è il valore del `yield`.

#canvas(height: 6.4cm)[
  #group(0.1cm, 0.2cm, 7.4cm, 6.0cm, [srotolamento al primo `yield`])
  #node(3.8cm, 0.85cm, 6.8cm, 0.75cm, [`(yield i)` → `Err(Yielded { 1, [] })`], fill: warm.lighten(85%))
  #carrow((3.8cm, 1.25cm), (3.8cm, 1.8cm))
  #node(3.8cm, 2.25cm, 6.8cm, 0.85cm, [`run_statements` del `while` aggiunge\ `While { from: 2 }`])
  #carrow((3.8cm, 2.7cm), (3.8cm, 3.25cm))
  #node(3.8cm, 3.7cm, 6.8cm, 0.85cm, [il corpo del `let` aggiunge\ `Body { [while, 'fine], 1, E1 }`])
  #carrow((3.8cm, 4.15cm), (3.8cm, 4.7cm))
  #node(3.8cm, 5.2cm, 6.8cm, 0.85cm, [la fiber aggiunge `Body { [let], 1, E0 }`;\ `resume` salva la lista, restituisce 1], fill: paper)

  #group(8.6cm, 0.2cm, 7.5cm, 6.0cm, [ripresa: il `resume` successivo])
  #node(12.35cm, 0.85cm, 6.9cm, 0.75cm, [`resume_frames(pending)`], fill: paper)
  #carrow((12.35cm, 1.25cm), (12.35cm, 1.8cm))
  #node(12.35cm, 2.6cm, 6.9cm, 1.4cm, [esegue ogni frame, dal più interno:\ `Body` → il resto del corpo da `from`\ `While` → il resto dell'iterazione, poi il ciclo])
  #carrow((12.35cm, 3.3cm), (12.35cm, 3.85cm))
  #node(12.35cm, 4.6cm, 6.9cm, 1.3cm, [nuovo `yield`: i frame esterni non ancora\ raggiunti si accodano al nuovo elenco;\ pila esaurita senza errori: `is_done = true`])
]

== La ripresa: `resume` e `resume_frames`

#estratto("base/fibers.rs")[
```rust
let frames = {
    let mut fiber = shared_fiber.0.write()?;
    if fiber.is_done { return Ok(LispExp::nil()); }
    std::mem::take(&mut fiber.pending)       // la pila esce dalla fiber...
};                                           // ...e il lock è rilasciato qui
if frames.is_empty() { /* is_done = true */ return Ok(LispExp::nil()); }
let outcome = resume_frames(frames, ctx);    // gira senza lock
let mut fiber = shared_fiber.0.write()?;
match outcome {
    Err(EvalError::Yielded { value, frames }) => { fiber.pending = frames; Ok(value) }
    Ok(value) => { fiber.is_done = true; Ok(value) }
    Err(other) => { fiber.is_done = true; fiber.pending.clear(); Err(other) }
}
```
]

`resume` toglie la pila dalla fiber e rilascia il lock prima di eseguirla: il
codice della fiber può quindi chiedere `fiber-done-p` di sé stessa senza
stallo. Un secondo `resume` che arriva mentre la fiber gira trova una pila
vuota e la segna come finita #rapporto("INT-9"). Un errore dentro una fiber la
termina: il punto di ripresa è andato perduto con lo srotolamento, e riprendere
vorrebbe dire ricominciare da un punto già superato.

#estratto("eval.rs")[
```rust
pub fn resume_frames<T>(frames: Vec<Frame<T>>, ctx: &T) -> Result<LispExp<T>, EvalError<T>> {
    let mut last = LispExp::nil();
    let mut outer = frames.into_iter();
    while let Some(frame) = outer.next() {
        let finished = match frame {
            Frame::Body { forms, from, env } =>
                run_statements(&forms, from, &env, ctx, true, last.clone(), /* … */),
            Frame::While { condition, forms, from, env } =>
                run_while(&condition, &forms, from, &env, ctx, last.clone()),
        };
        match finished {
            Ok(value) => last = value,
            Err(EvalError::Yielded { value, mut frames }) => {
                frames.extend(outer);          // i frame esterni non ancora raggiunti
                return Err(EvalError::Yielded { value, frames });
            }
            Err(other) => return Err(other),
        }
    }
    Ok(last)
}
```
]

Due dettagli rendono la ripresa equivalente all'esecuzione senza interruzioni:

- il valore di un blocco che non ha più istruzioni da eseguire è il valore
  prodotto dai blocchi al suo interno: per questo ogni frame riceve `last` come
  valore #emph[portato] (`carried`). Senza, `(fiber (progn (yield 1) 7))`
  risponderebbe `nil` alla seconda ripresa invece di `7`;
- se la ripresa di un blocco si sospende di nuovo, i frame esterni non ancora
  eseguiti vengono accodati alla nuova lista: sono ancora la strada per tornare
  indietro.

== L'esempio, frame per frame

La tabella segue la fiber dell'inizio del capitolo. E0 è l'ambiente della fiber,
E1 quello del `let`.

#ref-table(
  columns: (1.7cm, 1.5cm, 1fr),
  header: ([`resume`], [Valore], [`pending` dopo la chiamata]),
  [1º], [`1`], [`While { from: 2 }` · `Body { [while, 'fine], from: 1, E1 }` · `Body { [let], from: 1, E0 }`],
  [2º], [`2`], [Il `While` finisce l'iterazione (niente dopo il `yield`), rivaluta la condizione, rientra e si sospende di nuovo: `While { from: 2 }` più i due `Body` accodati, invariati.],
  [3º], [`fine`], [Il `While` trova la condizione falsa; il primo `Body` esegue `'fine` dall'indice 1; il secondo non ha più nulla e restituisce il valore portato, `fine`. Pila vuota: `is_done = true`.],
  [4º], [`nil`], [La fiber è finita: `resume` restituisce `nil` senza eseguire nulla.],
)

`fiber-done-p` è l'unico modo affidabile di distinguere una fiber finita da una
che ha fatto `yield` di `nil`.

// ===========================================================================
= Il carburante <carburante>
// ===========================================================================

Il carburante protegge dai cicli infiniti. Mentre un comando gira, l'editor non
legge la tastiera, quindi un `(while t ...)` scritto per sbaglio non potrebbe
essere interrotto: il carburante lo ferma dopo un numero stabilito di passi con
l'errore `OutOfFuel`. L'interprete fornisce il #emph[meccanismo] in `fuel.rs`;
quanto carburante dare e quando ricaricarlo è #emph[politica] dell'host.

== Il contatore e il budget

#canvas(height: 4.8cm)[
  #group(0.2cm, 0.3cm, 6.7cm, 4.2cm, [per processo])
  #node(3.55cm, 1.3cm, 6.0cm, 0.9cm, [`FuelMeter { budget: AtomicU32 }`\ il budget configurato, condiviso])
  #node(3.55cm, 3.2cm, 6.0cm, 1.1cm, [`begin()` → `FuelScope` (RAII)\ `grant(n)` · `arm_thread()` · `consume(n)`], fill: paper)

  #group(9.3cm, 0.3cm, 6.7cm, 4.2cm, [per thread (thread-local)])
  #node(12.65cm, 1.3cm, 6.0cm, 0.9cm, [`FUEL: Cell<u32>`\ quanto resta allo scope corrente])
  #node(12.65cm, 3.2cm, 6.0cm, 0.9cm, [`DEPTH: Cell<u32>`\ quanti `begin()` sono aperti])
  #carrow((6.55cm, 2.95cm), (9.65cm, 1.5cm))
  #note(7.0cm, 1.65cm)[`consume`: −n]
  #carrow((6.55cm, 3.35cm), (9.65cm, 3.35cm))
  #note(7.35cm, 3.45cm)[`begin`: +1]
  #note(9.7cm, 2.05cm)[`begin` a profondità 0 ricarica `FUEL` dal budget]
]

Il budget configurato è uno solo per processo, in un `AtomicU32` dentro il
`FuelMeter`. Il #emph[residuo] è invece una variabile per thread, per due
motivi: nessuna sincronizzazione sul percorso caldo (lo si tocca a ogni passo),
e un thread creato da `spawn` non può spendere il budget del thread che lo ha
creato, né vederselo ricaricare sotto i piedi.

== Le operazioni

#ref-table(
  columns: (3.4cm, 1fr),
  header: ([Operazione], [Effetto]),
  [`consume(n)`], [Toglie `n` dal residuo; se non basta restituisce `Exhausted`, che l'host converte in `EvalError::OutOfFuel`. Il residuo non scende mai sotto zero.],
  [`begin()` → `FuelScope`], [Apre uno scope: se `DEPTH` passa da 0 a 1 ricarica il residuo dal budget, altrimenti aumenta solo `DEPTH`. La guardia restituita chiude lo scope quando viene distrutta, anche in caso di `?` o di ritorno anticipato.],
  [`grant(n)`], [Alza il residuo ad #emph[almeno] `n`, senza abbassarlo mai. Serve alle pulizie di `unwind-protect`.],
  [`arm_thread()`], [Carica il budget nel residuo di un thread appena creato, che non ha uno scope in cui annidarsi.],
  [`set_remaining(n)`], [Fissa il residuo esattamente a `n`, alzandolo o abbassandolo: serve ai turni brevi dei worker dell'editor, ciascuno con la propria piccola quota.],
  [`measure(meter, f)` (solo test)], [Esegue `f` con un residuo praticamente illimitato e riporta quante unità ha speso: un numero esatto e riproducibile su ogni macchina, adatto a fissare regressioni nei test.],
)

Un thread che non apre mai uno scope parte comunque da `DEFAULT_FUEL`
(10 000 000 di passi, circa un secondo in release): è limitato anche senza la
collaborazione dell'host. Il numero è una protezione contro le fughe, non una
quota: il lavoro normale ne usa una frazione minima.

== Dove si paga

#ref-table(
  columns: (1fr, 1fr),
  header: ([Chi addebita], [Quanto]),
  [`eval_step_permitted`], [un'unità all'ingresso di ogni passo, prima di sapere che cosa farà],
  [`expect_list` (primitive di base che scorrono liste)], [un'unità per elemento: `length`, `mapcar`, `member` su una lista enorme si fermano in tempo],
  [primitive dell'editor], [alcune addebitano ciò che scorrono (testo, percorsi, candidati di completamento)],
)

Ciò che gira in Rust senza passare da questi punti non paga: il ciclo di
`dotimes` con un corpo vuoto, l'allocazione di `make-string`, il lavoro interno
di una primitiva #rapporto("INT-5").

Qualche conto, per farsi un'idea delle unità: `(+ 1 (* 2 3))` costa 5 unità
(la forma esterna, `1`, la forma interna, `2`, `3`); un giro di
`(while (< i n) (setq i (+ i 1)))` ne costa 7 (la condizione e i suoi due
argomenti, il `setq`, il `+` e i suoi due argomenti), quindi il budget
predefinito basta per circa un milione e mezzo di giri.

== Il residuo durante un comando

La figura segue il residuo durante un comando dell'editor che ne valuta un
altro al proprio interno (per esempio `eval-file` dentro un comando).

#canvas(height: 3.0cm)[
  #segment((0.4cm, 2.4cm), (15.8cm, 2.4cm), colour: dim)
  #note(15.0cm, 2.5cm)[tempo]
  #node(2.0cm, 0.5cm, 3.0cm, 0.6cm, [`begin()` · `DEPTH 0→1`])
  #note(0.8cm, 1.0cm)[`FUEL ← budget`]
  #node(6.6cm, 0.5cm, 3.0cm, 0.6cm, [`begin()` · `DEPTH 1→2`])
  #note(5.4cm, 1.0cm)[nessuna ricarica]
  #node(11.0cm, 0.5cm, 3.0cm, 0.6cm, [`drop` · `DEPTH 2→1`])
  #node(14.6cm, 0.5cm, 2.4cm, 0.6cm, [`drop` · `1→0`])
  #segment((0.6cm, 1.7cm), (15.6cm, 2.25cm), colour: accent)
  #note(6.0cm, 1.55cm, colour: accent)[il residuo scende soltanto]
]

Ricaricare solo al passaggio da 0 a 1 è ciò che impedisce a un programma di
procurarsi un budget nuovo rientrando nel valutatore a metà di un comando.

// ===========================================================================
= Thread e stato condiviso <thread>
// ===========================================================================

`(spawn (lambda () ...))` avvia un thread del sistema operativo che valuta le
forme del corpo della chiusura.

#estratto("eval.rs")[
```rust
let target_closure = eval(&args[0], env.clone(), ctx)?;
if let LispExp::Lambda(lambda_data) = target_closure {
    let lambda_clone = lambda_data.clone();
    let mut thread_ctx = ctx.clone();
    std::thread::spawn(move || {
        thread_ctx.begin_thread_evaluation();            // arma il carburante
        let thread_frame = Env::new_child(&lambda_clone.env);
        for exp in &lambda_clone.body {
            if let Err(err) = eval(exp, thread_frame.clone(), &mut thread_ctx) {
                thread_ctx.log_diagnostic(&format!("[LISP thread] {err:?}"));
                break;                                   // nessuno a cui restituirlo
            }
        }
    });
    Ok(EvalStep::Done(LispExp::form(vec![])))           // subito, una lista vuota
}
```
]

Il nuovo thread riceve un #emph[clone] del contesto e un ambiente figlio di
quello della chiusura. I parametri della lambda non vengono legati: il corpo
vede soltanto ciò che la chiusura ha catturato. Il primo errore interrompe il
thread e finisce nella diagnostica, perché non c'è nessuno a cui restituirlo.
`spawn` restituisce subito una forma vuota, che si stampa `()` #rapporto("INT-18").

#canvas(height: 4.3cm)[
  #group(0.1cm, 0.3cm, 7.1cm, 3.1cm, [thread che chiama `spawn`])
  #node(3.65cm, 1.2cm, 6.4cm, 0.8cm, [`(spawn (lambda () ...))` → `()` subito])
  #node(3.65cm, 2.5cm, 6.4cm, 0.8cm, [prosegue con il proprio `FUEL`, `DEPTH`, permesso])
  #group(9.0cm, 0.3cm, 7.1cm, 3.1cm, [thread nuovo])
  #node(12.55cm, 1.2cm, 6.4cm, 0.8cm, [`begin_thread_evaluation()` → `arm_thread()`])
  #node(12.55cm, 2.5cm, 6.4cm, 0.8cm, [valuta il corpo in `new_child(&lambda.env)`])
  #carrow((6.85cm, 1.2cm), (9.35cm, 1.2cm))
  #note(7.45cm, 0.75cm)[clone di `ctx`]
  #note(0.2cm, 3.6cm)[condiviso tra i due thread: gli ambienti (ogni tabella ha il suo `RwLock`), gli atomi, le fiber, l'obarray.\ Separato: carburante, profondità degli scope, permesso di `yield`.]
]

#ref-table(
  columns: (1fr, 1fr),
  header: ([Stato per thread], [Perché è separato]),
  [`FUEL`, `DEPTH`], [un budget appartiene a una sola linea di esecuzione],
  [`YIELD_PERMITTED`], [il permesso di un'istruzione su un thread non deve autorizzare un argomento su un altro],
  [lato editor: dati di `string-match`, cache delle espressioni regolari, contrassegno dei comandi per l'annullamento], [ognuno descrive ciò che sta facendo #emph[quel] thread],
)

Gli atomi (`make-atom`, `deref`, `reset`) sono il modo previsto di scambiare
valori mutabili tra thread: `deref` prende il lock di lettura, `reset` quello
di scrittura. Non esiste un aggiornamento atomico «leggi, calcola, scrivi»
#rapporto("INT-21"). Due thread che fanno `setq` sulla stessa variabile globale
non si danneggiano (ogni tabella ha il suo lock), ma non hanno alcuna garanzia
d'ordine.

// ===========================================================================
= Il contratto con l'host <contratto>
// ===========================================================================

== Il trait `LispContext`

```rust
pub trait LispContext: Clone + PartialEq + Debug + Send + Sync + 'static {
    fn consume_fuel(&self, amount: u32) -> Result<(), EvalError<Self>> { Ok(()) }
    fn log_diagnostic(&self, msg: &str) {}
    fn begin_unwind(&self) {}
    fn begin_thread_evaluation(&self) {}
    fn push_call_frame(&self, frame: &str) {}
    fn pop_call_frame(&self) {}
    fn call_frame_depth(&self) -> usize { 0 }
    fn truncate_call_frames(&self, depth: usize) {}
}
```

Ogni metodo ha un'implementazione vuota, così un host che non misura, non
registra e non tiene backtrace non paga nulla: i test dell'interprete usano
`T = ()`. Il contesto è passato per riferimento a ogni `eval` e a ogni
primitiva, ed è clonato soltanto da `spawn`; i vincoli `Send + Sync + 'static`
esistono proprio per `spawn`.

#ref-table(
  columns: (4.9cm, 1fr, 1fr),
  header: ([Metodo], [Quando lo chiama l'interprete], [Che cosa fa l'editor]),
  [`consume_fuel(n)`], [a ogni passo (`n = 1`) e dalle primitive che scorrono liste], [scala il `FuelMeter` del compartimento `Runtime`; `OutOfFuel` se non basta],
  [`log_diagnostic`], [errori nei thread di `spawn`, pulizie fallite, avvisi delle primitive], [scrive nel registro dell'editor (e nel file, se è attivo)],
  [`begin_unwind`], [prima delle pulizie di `unwind-protect`], [`grant(100 000)`],
  [`begin_thread_evaluation`], [all'inizio di uno `spawn`], [`arm_thread()`],
  [`push_call_frame`, `pop_call_frame`], [all'ingresso e all'uscita di una chiamata (@backtrace)], [una `Vec<String>` nel compartimento `Runtime`],
  [`call_frame_depth`, `truncate_call_frames`], [`catch`, `condition-case`, `unwind-protect`, `eval-string-safe`], [legge e accorcia la stessa `Vec`],
)

#canvas(height: 4.3cm)[
  #node(1.8cm, 2.1cm, 3.4cm, 4.0cm, [*valutatore*\ `eval.rs`], fill: paper)
  #node(14.4cm, 2.1cm, 3.4cm, 4.0cm, [*host*\ `T: LispContext`\ (`EditorState`\ nell'editor)], fill: accent.lighten(85%))
  #carrow((3.5cm, 0.55cm), (12.7cm, 0.55cm), label: [`consume_fuel(1)` a ogni passo], dy: -7pt)
  #carrow((3.5cm, 1.25cm), (12.7cm, 1.25cm), label: [`push_call_frame` / `pop_call_frame` a ogni chiamata], dy: -7pt)
  #carrow((3.5cm, 1.95cm), (12.7cm, 1.95cm), label: [`begin_unwind` prima delle pulizie di `unwind-protect`], dy: -7pt)
  #carrow((3.5cm, 2.65cm), (12.7cm, 2.65cm), label: [`begin_thread_evaluation` all'inizio di uno `spawn`], dy: -7pt)
  #carrow((3.5cm, 3.35cm), (12.7cm, 3.35cm), label: [`log_diagnostic` per errori di thread e avvisi], dy: -7pt)
  #carrow((12.7cm, 3.95cm), (3.5cm, 3.95cm), label: [primitive dell'host, registrate nella radice: `fn(&[LispExp], Arc<Env>, &T)`], dy: -7pt, colour: accent)
]

== L'avvio: `bootstrap_vm`

#canvas(height: 2.6cm)[
  #node(2.3cm, 0.8cm, 4.2cm, 0.9cm, [`Env::new_root()`\ con tabella proprietà])
  #node(7.3cm, 0.8cm, 4.2cm, 0.9cm, [`setup_base_env`\ 86 primitive])
  #node(12.6cm, 0.8cm, 5.0cm, 0.9cm, [script di verifica: aritmetica,\ fiber che si ferma e riparte])
  #carrow((4.4cm, 0.8cm), (5.2cm, 0.8cm))
  #carrow((9.4cm, 0.8cm), (10.1cm, 0.8cm))
  #note(0.4cm, 1.65cm)[se lo script non produce `vm-status-healthy`, `bootstrap_vm` restituisce un errore: un interprete rotto\ si scopre all'avvio, non al primo tasto. L'host aggiunge poi le proprie primitive alla stessa radice.]
]

Lo script calcola `(- (+ 5 5) 2)` e fa partire una fiber `(fiber (yield 100) 200)`,
che al primo `resume` deve restituire 100: una fiber che non si fermasse
darebbe 200, una che non partisse darebbe `nil`. Il risultato atteso è il
simbolo `vm-status-healthy`; altrimenti `bootstrap_vm` scrive il risultato nella
diagnostica e restituisce `UncorrectFunctionDefinition`. Lo script usa `setq`
nella radice, quindi le tre variabili che usa restano globali #rapporto("INT-19").

== Cosa l'interprete esporta

`lisp/mod.rs` rende pubblici, all'interno del crate: `eval`, `resume_frames`,
`Parser`, `form_to_data`, `Env`, `VARIABLE_DOCUMENTATION`, `LispExp`, `Lambda`,
`Frame`, `LispPrimitive`, `EvalError`, `LispContext`, `FuelMeter`, `FuelScope`,
`DEFAULT_FUEL`, `set_remaining`, `bootstrap_vm`, `setup_base_env`,
`call_callable`, `lisp_display`, `exact_arity`, `some_arguments` (e `measure`
nei test). Il crate `rsedit_core` riesporta verso l'esterno soltanto `Env`,
`LispContext`, `Parser` ed `eval`.

// ===========================================================================
= Le primitive di base <primitive-base>
// ===========================================================================

== Organizzazione

Una primitiva sta in `base/` se avrebbe lo stesso significato in un Lisp senza
editor: il criterio è poterla scrivere contro `T: LispContext` senza sapere che
cosa sia `T`. Ogni modulo ha una funzione `install(into: &Registry<T>)` che
registra le proprie primitive; `setup_base_env` è soltanto l'elenco dei moduli,
in ordine alfabetico.

#estratto("base/mod.rs")[
```rust
pub(crate) struct Registry<'a, T: LispContext> { env: &'a Arc<Env<T>> }
impl<T: LispContext> Registry<'_, T> {
    pub(crate) fn function(&self, name: &str, pointer: LispPrimitive<T>, doc: &'static str) {
        self.env.set_function(name.into(), LispExp::primitive(pointer, Some(doc.into())));
    }
}
```
]

#ref-table(
  columns: (auto, auto, 1fr),
  header: ([Modulo], [N.], [Primitive]),
  [`atoms`], [3], [`make-atom`, `deref`, `reset`],
  [`comparisons`], [7], [`=`, `<`, `>`, `<=`, `>=`, `max`, `min`],
  [`fibers`], [2], [`resume`, `fiber-done-p`],
  [`functions`], [8], [`funcall`, `eval`, `eval-string`, `eval-string-safe`, `throw`, `signal`, `function-doc`, `variable-doc`],
  [`inspect`], [6], [`boundp`, `symbol-value`, `function-arglist`, `all-functions`, `all-macros`, `all-variables`],
  [`lists`], [21], [`car`, `cdr`, `cons`, `list`, `nth`, `nthcdr`, `length`, `append`, `reverse`, `member`, `memq`, `assoc`, `assq`, `elt`, `mapcar`, `mapc`, `apply`, `identity`, `add-to-list`, `append-to-list`, `remove-from-list`],
  [`math`], [10], [`+`, `-`, `*`, `/`, `mod`, `%`, `abs`, `floor`, `1+`, `1-`],
  [`predicates`], [14], [`eq`, `eql`, `equal`, `null`, `not`, `consp`, `listp`, `stringp`, `numberp`, `symbolp`, `functionp`, `vectorp`, `zerop`, `atom`],
  [`strings`], [13], [`concat`, `substring`, `string=`, `string<`, `upcase`, `downcase`, `format`, `make-string`, `number-to-string`, `string-to-number`, `symbol-name`, `intern`, `split-string`],
  [`symbols`], [2], [`put`, `get`],
)

== Gli strumenti comuni

`base/mod.rs` contiene ciò che i moduli hanno in comune. Le macro generano le
primitive ripetitive, così una famiglia intera è una riga per primitiva:

#ref-table(
  columns: (3.0cm, 1fr),
  header: ([Macro], [Che primitiva genera]),
  [`predicate!`], [Un argomento; `t` se corrisponde a un pattern (o, con `not`, se non corrisponde). `consp`, `stringp`, `atom` e simili.],
  [`number_op!`, `string_op!`], [Un argomento numerico o stringa a cui si applica un metodo di `f64` o di `String` (`abs`, `floor`, `upcase`).],
  [`fold_numbers!`], [Almeno un argomento; piega i numeri con un metodo (`max`, `min`).],
  [`name_list!`], [Nessun argomento; la lista ordinata dei nomi di uno spazio (`all-functions`).],
  [`raise!`], [Due argomenti; restituisce un `Err` con quei due campi (`throw`, `signal`).],
  [`nil!`], [Abbreviazione di `LispExp::nil()`.],
)

Le funzioni di aiuto leggono gli argomenti in modo uniforme:

- `expect_number` ed `expect_string` restituiscono il valore o
  `WrongArgumentType` con il nome del tipo atteso;
- `expect_list` accetta una lista o `nil`, ne copia gli elementi in un vettore
  e #emph[addebita un'unità di carburante per elemento] (`charge`);
- `call_callable` chiama una lambda, una primitiva o un simbolo (@chiamate);
- `find_member` e `find_assoc` sono il cuore di `member`/`memq` e di
  `assoc`/`assq`: scorrono la lista con `expect_list` e confrontano con
  `PartialEq`, cioè con `equal` #rapporto("INT-16");
- `compare_chain` controlla una relazione su ogni coppia adiacente di numeri:
  `(< 1 2 3)` è vero se `1 < 2` e `2 < 3`.

== Semantiche da conoscere

#ref-table(
  columns: (4.4cm, 1fr),
  header: ([Primitive], [Comportamento]),
  [`+ - * /`], [Tutto in `f64`: `(/ 1 3)` vale `0.333…`; per una divisione intera si usa `floor`. `(- x)` cambia segno, `(/ x)` è il reciproco; dividere per zero è un `RuntimeMessage`, non un infinito.],
  [`mod`, `%`], [La stessa funzione: il segno del risultato segue il divisore, `(% -7 2)` vale 1.],
  [`car`, `cdr`], [Su `nil` restituiscono `nil`; su una lista puntata `cdr` restituisce la coda.],
  [`nth`, `elt`], [Indice fuori dai limiti o negativo: `nil`. `elt` funziona anche su vettori e stringhe.],
  [`length`, `reverse`], [Liste, vettori e stringhe (in caratteri).],
  [`append`], [Concatena; l'ultimo argomento diventa la coda così com'è.],
  [`mapcar`, `mapc`, `apply`], [Accettano una lambda, una primitiva o un simbolo che nomina una funzione; chiamano con `call_callable`.],
  [`substring`], [Indici in caratteri, negativi contati dalla fine.],
  [`format`], [`%s`/`%S` (come `lisp_display`), `%d` (tronca a intero), `%f`, `%%`; meno argomenti del necessario è un errore. Riconosce anche le sequenze `\n`, `\t`, `\r` scritte nella stringa di formato e rifiuta le altre #rapporto("INT-12").],
  [`make-string`], [`(make-string N "x")` ripete il #emph[primo carattere] della stringa N volte.],
  [`string-to-number`], [0 per un testo non numerico; usa `f64::from_str`.],
  [`split-string`], [Senza separatori divide sugli spazi bianchi e scarta i pezzi vuoti; con una stringa vuota divide in caratteri; altrimenti divide sulle occorrenze letterali della stringa data (`"::"` è un solo separatore di due caratteri, non due separatori).],
  [`intern`, `symbol-name`], [Da stringa a simbolo e ritorno.],
  [`eval`, `eval-string`], [Valutano nell'ambiente del #emph[chiamante]: `(let ((x 5)) (eval 'x))` vale 5. `eval-string` valuta solo la prima espressione.],
  [`boundp`, `symbol-value`], [Guardano lo spazio delle variabili dall'ambiente del chiamante.],
  [`function-arglist`], [La lista dei parametri come stringhe, con `&optional` e `&rest`.],
  [`put`, `get`], [La tabella delle proprietà nella radice (@proprieta).],
  [`add-to-list`, `append-to-list`, `remove-from-list`], [Ricevono il #emph[simbolo] della lista, calcolano una nuova lista e la legano con `set_variable` nell'ambiente del chiamante.],
)

== Ricetta: aggiungere una primitiva di base

+ Scegli il modulo per argomento. Se nessuno va bene, crea un file, dichiaralo in
  `base/mod.rs` e aggiungi la sua `install` a `setup_base_env`.
+ Scrivi la docstring come `const`. La prima riga #emph[deve] essere la firma
  seguita da una frase, nella forma `(nome ARG &optional ALTRO): cosa fa.`: è
  ciò che mostra `describe-function`, e i test sulla documentazione lo
  verificano.
+ Scrivi la funzione con la firma di `LispPrimitive<T>`. Controlla l'arità con
  `exact_arity` o `some_arguments`, i tipi restituendo `WrongArgumentType`.
+ Restituisci le liste con `LispExp::proper_list`, mai con `LispExp::form`.
+ Se scorri una lista, usa `expect_list` (o addebita con `ctx.consume_fuel`):
  altrimenti un ciclo che chiama la primitiva sfugge al limite.
+ Se allochi in base a un numero ricevuto dall'utente, limita il numero.
+ Per un errore con un nome proprio, restituisci `EvalError::Signal`.
+ Registra con `into.function(nome, puntatore, DOC)` nella `install` del modulo
  e aggiungi i test in `lisp/tests/`.

// ===========================================================================
= Differenze da Emacs Lisp <differenze>
// ===========================================================================

rsedit prende in prestito nomi e forme da Emacs Lisp, ma non è Emacs Lisp. La
tabella raccoglie le differenze osservate eseguendo l'interprete; chi porta
codice da Emacs deve conoscerle.

#ref-table(
  columns: (3.3cm, 1fr, 1fr),
  header: ([Argomento], [rsedit], [Emacs Lisp]),
  [Scope], [Lessicale per tutte le variabili, anche quelle di `defvar`.], [Dinamico per le variabili speciali (`defvar`), lessicale altrove.],
  [`setq` su un nome nuovo dentro una funzione], [Crea un legame locale alla chiamata.], [Crea una variabile globale.],
  [`defun`/`defvar` dentro una funzione], [Locali alla chiamata.], [Globali.],
  [`if` con più forme «altrimenti»], [Valuta solo la prima.], [Le valuta tutte, come un `progn`.],
  [Valore di `while`], [L'ultima forma eseguita dal corpo.], [Sempre `nil`.],
  [`dolist` con risultato], [La variabile vale ancora l'ultimo elemento.], [La variabile vale `nil`.],
  [Backquote annidato], [Un solo livello.], [Livelli annidati gestiti.],
  [Numeri], [Solo `f64`; niente `1e20`, niente interi separati.], [Interi e virgola mobile distinti; notazione esponenziale.],
  [Divisione], [`(/ 1 3)` vale `0.333...`.], [Tra interi è intera: `(/ 1 3)` vale `0`.],
  [`%`], [Come `mod`: il segno segue il divisore.], [Resto: il segno segue il dividendo.],
  [Caratteri], [Nessun tipo carattere né sintassi `?a`; `make-string` prende una stringa e ne usa il primo carattere.], [`?a` è l'intero 97; `make-string` prende un carattere.],
  [Lambda citata], [`'(lambda ...)` non è chiamabile.], [Chiamabile.],
  [Condizioni], [Nessuna gerarchia né `define-error`; `error` e `t` intercettano qualunque errore.], [Gerarchie dichiarate.],
  [`format` con `%S`], [Come `%s`: `(format "%S" "a")` vale `"a"`.], [Stampa leggibile: le virgolette restano.],
  [`eval-string`], [Valuta solo la prima espressione.], [(non esiste; si usa `read` + `eval`)],
  [`add-to-list`], [Accetta più elementi, nessun `APPEND`; scrive nell'ambiente del chiamante.], [Un elemento, poi `APPEND` e `COMPARE-FN`; aggiorna il simbolo globale.],
  [Espressioni regolari], [Sintassi del crate `regex` di Rust (`(...)`), senza lookaround.], [Sintassi Emacs (`\(...\)`).],
  [Macro], [Niente `gensym`, niente docstring; espanse a ogni valutazione.], [`gensym`, docstring, espansione memorizzata dal compilatore.],
  [`yield`, `fiber`, `spawn`, atomi], [Presenti (@fiber, @thread).], [Assenti.],
)

// ===========================================================================
= Estendere l'interprete <estendere>
// ===========================================================================

== Una nuova forma speciale

Una forma speciale serve solo quando gli argomenti non devono essere valutati
prima e una macro non basta, per esempio quando la forma deve intercettare
errori o sospensioni. Negli altri casi è preferibile una macro in Lisp o una
primitiva.

+ Aggiungi un ramo in `eval_special_form_or_call_step`. Il ramo riceve gli
  argomenti non valutati, l'ambiente, il contesto e il permesso di sospendersi.
+ Decidi la #emph[posizione di coda]: se l'ultima cosa da fare è valutare una
  forma il cui valore è il risultato, restituisci `TailCall`. Se invece la forma
  deve vedere un errore che risale, valuta dentro il ramo con `eval`.
+ Decidi la sospendibilità: un corpo che passa da `eval_body_step` o
  `run_statements` diventa sospendibile senza altro lavoro, perché sono loro a
  scrivere i frame. Altrimenti un `yield` interno viene rifiutato, che è la
  scelta sicura.
+ Se intercetti errori, annota `call_frame_depth` prima e chiama
  `truncate_call_frames` dopo.
+ Se serve un nuovo errore di forma malformata, aggiungilo a `EvalError`; per
  dargli una condizione con nome, aggiornalo in `error_symbol` e, se porta dati,
  in `error_data`.
+ Aggiorna questo manuale: la tabella delle forme speciali, quella delle
  posizioni di coda e, se cambia, quella delle posizioni di `yield`.

== Una nuova variante di `LispExp`

È la modifica più invasiva. Vanno aggiornati: `PartialEq` e `Debug` in
`lispexp.rs`, lo smistamento in `eval_step_permitted`, i predicati e la stampa
nelle primitive di base (`lisp_display`, `format`, `eq`), e --- se la variante
è un contenitore --- `form_to_data`, `data_to_form` ed `eval_backquote`. Prima
di farlo, conviene chiedersi se un `Atom`, una `Map` o un dato agganciato
dall'host non bastino.

== I test

I test stanno in `lisp/tests/` e girano con il contesto `()`.

#ref-table(
  columns: (1fr, 1fr),
  header: ([File], [Cosa fissano]),
  [`eval_tests.rs`], [le forme speciali e i loro errori di forma],
  [`handshake.rs`], [`bootstrap_vm` e il limite ai cicli con un host fittizio che misura da sé il carburante; `add-to-list` e simili],
  [`lexical_context.rs`], [catena degli ambienti, `setq`, `let`, scope lessicale, chiusure],
  [`nonlocal_exit_tests.rs`], [`catch`, `condition-case`, `unwind-protect`],
  [`yield_tests.rs`, `fiber_tests.rs`], [sospensione e ripresa],
  [`fuel.rs`], [scope, ricariche, concessioni],
  [`thread_tests.rs`], [`spawn`, atomi, carburante per thread],
  [`backtrace_tests.rs`], [il protocollo dei frame],
  [`lisp_core_compliance_tests.rs`], [comportamenti allineati a Emacs Lisp],
  [`parser_tests.rs`, `lexer_tests.rs`], [lettura e token],
  [`base_env_tests.rs`, `documentation_tests.rs`, `symbol_tests.rs`], [primitive di base, docstring, proprietà],
)

// ===========================================================================
= Appendice
// ===========================================================================

== Glossario

/ Forma: una lista di codice (`LispExp::Form`), letta dal lettore e smistata dal valutatore.
/ Dato: un valore, in particolare una lista di `Cons`.
/ Primitiva: una funzione scritta in Rust e registrata in un `Env`.
/ Forma speciale: un costrutto riconosciuto per nome dal valutatore, i cui argomenti non sono valutati in anticipo.
/ Istruzione: una forma di un corpo il cui valore viene scartato; l'unico punto, insieme alle chiamate in coda, in cui una fiber può sospendersi.
/ Posizione di coda: una forma il cui valore è il valore della forma che la contiene; valutata dal trampolino senza far crescere lo stack.
/ Trampolino: il ciclo di `eval` che esegue le chiamate in coda.
/ Carburante: il numero di passi che uno scope può ancora eseguire.
/ Frame (di fiber): la descrizione di un blocco sospeso, cioè di che cosa resta da eseguire.
/ Frame (di backtrace): una voce della pila delle chiamate tenuta dall'host.
/ Host: il programma che incorpora l'interprete e fornisce `T: LispContext`.

== Mantenere aggiornato questo manuale

Questo file è pensato per crescere con il codice. Quando cambi l'interprete,
controlla le sezioni che descrivono la parte modificata:

- nuova forma speciale → @valutatore (tabelle delle forme e delle posizioni di
  coda) e @fiber (posizioni di `yield`);
- nuova variante di `EvalError` con condizione propria → @errori;
- nuova primitiva di base o nuovo modulo → @primitive-base;
- nuovo metodo di `LispContext` → @contratto;
- nuovo stato per thread → @thread;
- comportamento allineato o reso diverso da Emacs → @differenze;
- difetto corretto → aggiorna la descrizione e togli il rimando al rapporto,
  poi togli la scheda da `rapporto-problemi.typ`.

Gli estratti di codice vanno riletti quando cambia la funzione da cui vengono:
sono semplificati, ma ogni riga deve corrispondere a una riga del sorgente.

I diagrammi usano gli aiuti di `style.typ`: `canvas`, `node`, `arrow`, `carrow`,
`segment`, `group`, `note` e, per le sequenze, `lane`, `msg`, `self-msg` e
`seq-note`; le tabelle di riferimento usano `ref-table`. Le coordinate sono in
centimetri dall'angolo in alto a sinistra del riquadro; `node` riceve il
#emph[centro] della casella, `group` il suo angolo in alto a sinistra. Dopo ogni
modifica conviene compilare e guardare le pagine: Typst segnala gli errori di
sintassi, non le etichette che si sovrappongono.
