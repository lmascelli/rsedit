#[cfg(test)]
mod performance_tests {
    use crate::{
        buffer::{
            Buffer,
            buffer_trait::BufferTrait,
            gap_buffer::GapBuffer,
        },
        ui::*,
    };
    use std::time::Instant;
    // Assicurati di importare i moduli giusti per il tuo GapBuffer e LayoutNode

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
            massive_string.push_str("Questa è una riga di test abbastanza lunga per fare volume.\n");
        }
        
        // 2. Carichiamo nel GapBuffer (il caricamento iniziale può essere lento, non lo misuriamo qui)
        let buf = GapBuffer::from(massive_string.as_str());
        
        // 3. Il vero Benchmark: estrarre le righe da 500.000 a 500.050 (il centro del file)
        let start = Instant::now();
        let viewport_lines = buf.get_lines(500_000, 500_050);
        let duration = start.elapsed();
        
        println!("Tempo per estrarre 50 righe da un file di {N}: {:?}", duration);
        
        // Questo è il miracolo dell'iterazione che si ferma presto (early break).
        // Deve impiegare meno di 2 o 3 millisecondi.
        assert!(duration.as_millis() < 5, "Il rendering della viewport non scala bene!");
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
                left: Box::new(LayoutNode::Leaf(Window { id: 1, buffer_name: "buf1".into(), scroll_x: 0, scroll_y: 0 })),
                right: Box::new(LayoutNode::Leaf(Window { id: 2, buffer_name: "buf2".into(), scroll_x: 0, scroll_y: 0 })),
            }),
            right: Box::new(LayoutNode::Split {
                orientation: Orientation::Vertical,
                ratio: 0.5,
                left: Box::new(LayoutNode::Leaf(Window { id: 3, buffer_name: "buf3".into(), scroll_x: 0, scroll_y: 0 })),
                right: Box::new(LayoutNode::Leaf(Window { id: 4, buffer_name: "buf4".into(), scroll_x: 0, scroll_y: 0 })),
            }),
        };

        // Populate the buffer hashmap with the updated struct fields
        let mut buffers = std::collections::HashMap::new();
        
        buffers.insert("buf1".into(), Buffer { 
            text: GapBuffer::from("Testo 1\n"), 
            name: "buf1".into(), 
            current_mode: "fundamental".into(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
        });
        
        buffers.insert("buf2".into(), Buffer { 
            text: GapBuffer::from("Testo 2\n"), 
            name: "buf2".into(), 
            current_mode: "fundamental".into(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
        });
        
        buffers.insert("buf3".into(), Buffer { 
            text: GapBuffer::from("Testo 3\n"), 
            name: "buf3".into(), 
            current_mode: "fundamental".into(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
        });
        
        buffers.insert("buf4".into(), Buffer { 
            text: GapBuffer::from("Testo 4\n"), 
            name: "buf4".into(), 
            current_mode: "fundamental".into(),
            file_path: None,
            is_modified: false,
            local_keymap: None,
        });

        let screen_rect = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let mut out_views = Vec::new();

        let start = std::time::Instant::now();
        // Simulate 100 frames of rendering
        for _ in 0..100 {
            out_views.clear();
            root.compute_tiled_views(screen_rect.clone(), 1, &buffers, &mut out_views);
        }
        let duration = start.elapsed();

        println!("Tempo medio per ricalcolare un layout a 4 viste: {:?}", duration / 100);
        
        // A single tree recalculation should take fractions of a millisecond.
        assert!(duration.as_millis() / 100 < 20, "Il motore di layout è troppo pesante!");
    }
}
