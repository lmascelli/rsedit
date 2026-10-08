//! Debug mode's reader: where each form was found.
#[cfg(test)]
mod tests {
    use crate::lisp::{LispExp, Location, Parser, SourceMap};
    use std::sync::Arc;

    fn read_located(text: &str, first_line: u32) -> (LispExp<()>, Arc<SourceMap>) {
        let map = Arc::new(SourceMap::default());
        let mut parser = Parser::with_source_map(text, map.clone(), "test.lisp", first_line);
        (parser.next().expect("reads"), map)
    }

    fn at(line: u32, column: u32, end_line: u32, end_column: u32) -> Option<Location> {
        Some(Location {
            file: "test.lisp".into(),
            line,
            column,
            end_line,
            end_column,
        })
    }

    fn form(exp: &LispExp<()>) -> &Arc<Vec<LispExp<()>>> {
        match exp {
            LispExp::Form(form) => form,
            other => panic!("not a form: {other:?}"),
        }
    }

    #[test]
    fn every_list_is_found_at_its_open_paren_to_its_close() {
        let (outer, map) = read_located("(defun f (x)\n  (when x\n    (g  x)))", 1);
        let outer_form = form(&outer);
        assert_eq!(map.locate(outer_form), at(1, 1, 3, 12));
        assert_eq!(map.locate(form(&outer_form[2])), at(1, 10, 1, 12));
        let when = form(&outer_form[3]);
        assert_eq!(map.locate(when), at(2, 3, 3, 11));
        assert_eq!(map.locate(form(&when[2])), at(3, 5, 3, 10));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let (outer, map) = read_located("(è (à))", 1);
        assert_eq!(map.locate(form(&form(&outer)[1])), at(1, 4, 1, 6));
    }

    #[test]
    fn a_wrapped_text_keeps_its_own_line_numbers() {
        // What eval_file reads: the file behind a line of its own.
        let (outer, map) = read_located("(progn\n(a)\n  (b))", 0);
        let body = form(&outer);
        assert_eq!(map.locate(form(&body[1])), at(1, 1, 2, 6));
        assert_eq!(map.locate(form(&body[2])), at(2, 3, 2, 5));
    }

    #[test]
    fn a_clone_is_found_too_and_a_freed_form_is_not() {
        let (outer, map) = read_located("(a b)", 1);
        let kept = form(&outer).clone();
        assert_eq!(map.locate(&kept), at(1, 1, 1, 5));
        drop(outer);
        drop(kept);
        // A form built afterwards is not mistaken for the freed one.
        let fresh: LispExp<()> = LispExp::form(vec![LispExp::nil()]);
        assert_eq!(map.locate(form(&fresh)), None);
    }

    #[test]
    fn the_nominal_reader_records_nothing() {
        let map = SourceMap::default();
        let read: LispExp<()> = Parser::new("(a (b))").next().unwrap();
        assert_eq!(map.locate(form(&read)), None);
    }
}