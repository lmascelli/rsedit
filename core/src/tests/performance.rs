#[cfg(test)]
mod performance_tests {
    use crate::{
        buffer::{Buffer, buffer_trait::BufferTrait, gap_buffer::GapBuffer},
        editor::{MajorMode, create_global_env},
        input::{KeyCode, KeyEvent, KeyModifiers},
        lisp::{LispExp, eval},
        ui::*,
    };
    use std::sync::{Arc, RwLock};
    use std::time::Instant;
    use std::thread;
    
    #[test]
    fn benchmark_gap_buffer_rapid_typing() {
        let mut buf = GapBuffer::default();

        let start = Instant::now();
        // Simuliamo un utente che digita 100.000 caratteri di fila
        for _ in 0..100_000 {
            buf.insert('x');
            // Inseriamo a capo ogni 80 caratteri
            if buf.len() % 80 == 0 {
                buf.insert('\n');
            }
        }
        let duration = start.elapsed();

        println!("Tempo per 100.000 inserimenti: {:?}", duration);
        // Su un PC moderno, questo dovrebbe richiedere meno di 10 millisecondi.
        // Impostiamo un limite conservativo di 50ms per far fallire il test se introduciamo regressioni.
        assert!(duration.as_millis() < 50, "Il GapBuffer è troppo lento!");
    }

    #[test]
    fn benchmark_massive_file_viewport() {
        // 1. Setup: Creiamo una stringa di 1 milione di righe
        let N = 10_000;
        let mut massive_string = String::with_capacity(20_000_000);
        for i in 0..N {
            massive_string
                .push_str("Questa è una riga di test abbastanza lunga per fare volume.\n");
        }

        // 2. Carichiamo nel GapBuffer (il caricamento iniziale può essere lento, non lo misuriamo qui)
        let buf = GapBuffer::from(massive_string.as_str());

        // 3. Il vero Benchmark: estrarre le righe da 500.000 a 500.050 (il centro del file)
        let start = Instant::now();
        let viewport_lines = buf.get_lines(500_000, 500_050);
        let duration = start.elapsed();

        println!(
            "Tempo per estrarre 50 righe da un file di {N}: {:?}",
            duration
        );

        // Questo è il miracolo dell'iterazione che si ferma presto (early break).
        // Deve impiegare meno di 2 o 3 millisecondi.
        assert!(
            duration.as_millis() < 5,
            "Il rendering della viewport non scala bene!"
        );
        assert_eq!(viewport_lines.len(), 50);
    }

    #[test]
    fn benchmark_layout_computation() {
        // Setup: a complex tree (screen split into 4 views)
        let mut root = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            left: Box::new(LayoutNode::Split {
                orientation: Orientation::Vertical,
                ratio: 0.5,
                left: Box::new(LayoutNode::Leaf(Window {
                    id: 1,
                    buffer_name: "buf1".into(),
                    scroll_x: 0,
                    scroll_y: 0,
                })),
                right: Box::new(LayoutNode::Leaf(Window {
                    id: 2,
                    buffer_name: "buf2".into(),
                    scroll_x: 0,
                    scroll_y: 0,
                })),
            }),
            right: Box::new(LayoutNode::Split {
                orientation: Orientation::Vertical,
                ratio: 0.5,
                left: Box::new(LayoutNode::Leaf(Window {
                    id: 3,
                    buffer_name: "buf3".into(),
                    scroll_x: 0,
                    scroll_y: 0,
                })),
                right: Box::new(LayoutNode::Leaf(Window {
                    id: 4,
                    buffer_name: "buf4".into(),
                    scroll_x: 0,
                    scroll_y: 0,
                })),
            }),
        };

        // Populate the buffer hashmap with the updated struct fields
        let mut buffers = std::collections::HashMap::new();

        buffers.insert(
            "buf1".into(),
            Arc::new(RwLock::new(Buffer {
                text: GapBuffer::from("Testo 1\n"),
                name: "buf1".into(),
                current_mode: "fundamental".into(),
                file_path: None,
                is_modified: false,
                local_keymap: None,
            })),
        );

        buffers.insert(
            "buf2".into(),
            Arc::new(RwLock::new(Buffer {
                text: GapBuffer::from("Testo 2\n"),
                name: "buf2".into(),
                current_mode: "fundamental".into(),
                file_path: None,
                is_modified: false,
                local_keymap: None,
            })),
        );

        buffers.insert(
            "buf3".into(),
            Arc::new(RwLock::new(Buffer {
                text: GapBuffer::from("Testo 3\n"),
                name: "buf3".into(),
                current_mode: "fundamental".into(),
                file_path: None,
                is_modified: false,
                local_keymap: None,
            })),
        );

        buffers.insert(
            "buf4".into(),
            Arc::new(RwLock::new(Buffer {
                text: GapBuffer::from("Testo 4\n"),
                name: "buf4".into(),
                current_mode: "fundamental".into(),
                file_path: None,
                is_modified: false,
                local_keymap: None,
            })),
        );

        let screen_rect = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let mut out_views = Vec::new();

        let start = std::time::Instant::now();
        // Simulate 100 frames of rendering
        for _ in 0..100 {
            out_views.clear();
            root.compute_tiled_views(screen_rect.clone(), 1, &buffers, &mut out_views);
        }
        let duration = start.elapsed();

        println!(
            "Tempo medio per ricalcolare un layout a 4 viste: {:?}",
            duration / 100
        );

        // A single tree recalculation should take fractions of a millisecond.
        assert!(
            duration.as_millis() / 100 < 20,
            "Il motore di layout è troppo pesante!"
        );
    }

    // Helper per generare un evento tastiera pulito
    fn char_event(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers {
                ctrl: false,
                alt: false,
                shift: false,
                caps_lock_as_ctrl: false,
            },
        }
    }

    fn ctrl_event(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers {
                ctrl: true,
                alt: false,
                shift: false,
                caps_lock_as_ctrl: false,
            },
        }
    }

    #[test]
    fn benchmark_native_typing_routing() {
        // Usa la tua funzione per creare l'ambiente pulito
        let (mut state, env) =
            create_global_env::<GapBuffer>().expect("Failed to create the global env");

        let start = Instant::now();
        // Simuliamo l'utente che digita 10.000 caratteri normali (nessun hook lisp)
        for _ in 0..10_000 {
            state.handle_key_event(char_event('a'), &env);
        }
        let duration = start.elapsed();

        println!(
            "Tempo medio per smistare e inserire un singolo carattere (Nativo): {:?}",
            duration / 10_000
        );

        // Questo è puro codice Rust (lookup fallito nella keymap -> inserimento nel gap buffer).
        // Dovrebbe impiegare pochissimi microsecondi per singolo carattere.
        assert!(
            duration.as_millis() < 50,
            "Il routing degli eventi nativi ha introdotto lag!"
        );
    }

    #[test]
    fn benchmark_lisp_bound_key_latency() {
        let (mut state, env) =
            create_global_env::<GapBuffer>().expect("Failed to create the global env");

        // Setup manuale di una Major Mode fittizia per il test
        let mut test_mode = MajorMode::new("test-mode");

        // Creiamo una funzione Lisp "dummy" velocissima (es. una lambda vuota o che fa un'addizione)
        // (lambda () (+ 1 1))
        let dummy_action = LispExp::list(vec![
            LispExp::symbol("lambda".into()),
            LispExp::list(vec![]),
            LispExp::list(vec![
                LispExp::symbol("+".into()),
                LispExp::number(1.0),
                LispExp::number(1.0),
            ]),
        ]);

        // Mappiamo Ctrl+T alla nostra dummy action
        test_mode.keymap.insert(ctrl_event('t'), dummy_action);
        state
            .mode_registry
            .write()
            .expect("Failed to acquire write lock on mode_registry")
            .insert("test-mode".to_string(), test_mode);

        let views = state.compose_layout(80, 24);

        // Forziamo il buffer corrente in test-mode
        if let Some(focused_win) = views.iter().find(|v| v.is_focused) {
            // Adatta alla tua struct
            if let Some(buffer) = state
                .buffers
                .read()
                .expect("Failed to acquire read lock on buffers")
                .get(&focused_win.buffer_name)
            {
                buffer
                    .write()
                    .expect("Failed to acquire write lock on buffer")
                    .current_mode = "test-mode".to_string();
            }
        }

        let start = Instant::now();
        // Simuliamo la pressione di Ctrl+T per 1.000 volte
        for _ in 0..1_000 {
            state.handle_key_event(ctrl_event('t'), &env);
        }
        let duration = start.elapsed();

        println!(
            "Tempo medio per lookup e valutazione Lisp semplice: {:?}",
            duration / 1_000
        );

        // Anche passando per l'evaluator Lisp, una chiamata semplice deve stare
        // matematicamente sotto il millisecondo (preferibilmente decimi di millisecondo).
        assert!(
            (duration.as_millis() / 1_000) < 1,
            "L'overhead del Lisp evaluator sui tasti è troppo alto!"
        );
    }

    #[test]
    fn benchmark_hook_simulation() {
        // Questo test ci proteggerà in futuro. Quando implementerai `post-command-hook`,
        // Lisp dovrà iterare su una lista di funzioni ogni volta che premi un tasto.
        // Simuliamo l'esecuzione di una "coda" di 5 comandi consecutivi.

        let (mut state, env) =
            create_global_env::<GapBuffer>().expect("Failed to create the global envs");

        // Simuliamo un hook che esegue 5 valutazioni
        let hook_code = LispExp::list(vec![
            LispExp::symbol("progn".into()),
            LispExp::list(vec![
                LispExp::symbol("setq".into()),
                LispExp::symbol("a".into()),
                LispExp::number(1.0),
            ]),
            LispExp::list(vec![
                LispExp::symbol("setq".into()),
                LispExp::symbol("b".into()),
                LispExp::number(2.0),
            ]),
            LispExp::list(vec![
                LispExp::symbol("setq".into()),
                LispExp::symbol("c".into()),
                LispExp::number(3.0),
            ]),
            LispExp::list(vec![
                LispExp::symbol("setq".into()),
                LispExp::symbol("d".into()),
                LispExp::number(4.0),
            ]),
            LispExp::list(vec![
                LispExp::symbol("setq".into()),
                LispExp::symbol("e".into()),
                LispExp::number(5.0),
            ]),
        ]);

        let start = Instant::now();
        // Eseguiamo l'hook 1.000 volte (come se avessimo premuto 1.000 tasti)
        for _ in 0..1_000 {
            let _ = eval(&hook_code, env.clone(), &mut state);
        }
        let duration = start.elapsed();

        println!(
            "Tempo medio esecuzione catena di Hook (5 exp): {:?}",
            duration / 1_000
        );

        // Eseguire 5 istruzioni in Lisp a ogni pressione di tasto DEVE essere impercettibile.
        assert!(
            (duration.as_millis() / 1_000) < 2,
            "Il sistema di Hook farà laggare l'editor!"
        );
    }

    #[test]
    fn benchmark_concurrent_stress_test() {
        // 1. Setup the global thread-safe environment
        let (state, _env) = create_global_env::<GapBuffer>().unwrap();
        
        // Pre-create a secondary buffer for our background worker
        state.new_buffer("*logs*", None);

        let start_time = Instant::now();

        // 2. Clone the EditorState for our threads.
        // Because of our Arc-based architecture, this is instantaneous (O(1)) 
        // and points to the exact same memory in RAM.
        let state_ui = state.clone();
        let state_writer_1 = state.clone();
        let state_writer_2 = state.clone();

        // THREAD 1: The UI Engine (Aggressive Reader)
        let ui_thread = thread::spawn(move || {
            let mut views_computed = 0;
            // Simulate 1,000 frames of rendering
            for _ in 0..1000 {
                // compose_layout locks the buffer map and specific buffers in READ mode
                let _views = state_ui.compose_layout(120, 40);
                views_computed += 1;
            }
            views_computed
        });

        // THREAD 2: Lisp Macro Execution (Aggressive Writer on *scratch*)
        let writer_thread_1 = thread::spawn(move || {
            let scratch_buf = state_writer_1.get_buffer("*scratch*");
            for _ in 0..10_000 {
                // Locks ONLY the *scratch* buffer in WRITE mode
                state_writer_1.mutate_buffer(scratch_buf.clone(), |buf| {
                    buf.text.insert('a');
                });
            }
        });

        // THREAD 3: Background Language Server/Logger (Aggressive Writer on *logs*)
        let writer_thread_2 = thread::spawn(move || {
            let logs_buf = state_writer_2.get_buffer("*logs*");
            for _ in 0..10_000 {
                // Locks ONLY the *logs* buffer in WRITE mode
                state_writer_2.mutate_buffer(logs_buf.clone(), |buf| {
                    buf.text.insert('L');
                });
            }
        });

        // 3. Wait for all threads to finish
        let views_rendered = ui_thread.join().unwrap();
        writer_thread_1.join().unwrap();
        writer_thread_2.join().unwrap();

        let duration = start_time.elapsed();
        println!(
            "Concurrency Stress Test Finished in: {:?} (Rendered {} frames)",
            duration, views_rendered
        );

        // 4. Verification: Did all the data survive the race conditions?
        let scratch_guard = state.get_buffer("*scratch*").read().unwrap().text.len();
        let logs_guard = state.get_buffer("*logs*").read().unwrap().text.len();

        // The scratch buffer should have its initial standard library output + 10,000 'a's
        assert!(scratch_guard >= 10_000, "Scratch buffer lost data due to race conditions!");
        assert_eq!(logs_guard, 10_000, "Logs buffer lost data due to race conditions!");
        
        // If the architecture is healthy, resolving 30,000 heavy concurrent operations 
        // should take comfortably under 100 milliseconds in a release build.
        assert!(duration.as_millis() < 200, "Lock contention is too high, the editor will lag!");
    }    
}
