use super::{
    ConsCell, ConsIter, FiberState, Lambda, LispContext, LispPrimitive, SharedAtom, SharedFiber,
};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, RwLock},
};

// --------------------------------  LispExp  ----------------------------------

#[derive(Clone, PartialEq)]
// Primitive comparison has no meaning and will probably never done
#[allow(unpredictable_function_pointer_comparisons)]
pub enum LispExp<T: LispContext> {
    /// A *syntax* node: the vector-backed form the reader produces and
    /// `eval` dispatches on. Never a runtime value -- a list of *data* is
    /// a `Cons` chain, and the two are not interchangeable.
    Form(Arc<Vec<LispExp<T>>>),

    /// A *data* list. `nil` terminates a proper list; anything else
    /// terminates an improper (dotted) one.
    Cons(Arc<ConsCell<T>>),

    Vector(Arc<Vec<LispExp<T>>>),
    Map(Arc<HashMap<String, LispExp<T>>>),
    Number(f64),
    Symbol(Arc<String>),
    String(Arc<String>),
    Lambda(Arc<Lambda<T>>),
    Primitive {
        pointer: LispPrimitive<T>,
        doc: Arc<Cow<'static, str>>,
    },
    Atom(SharedAtom<T>),
    Fiber(SharedFiber<T>),
}

/// `LispExp` prints as Lisp source, not as a Rust value. The derived
/// `Debug` was tolerable while lists were vectors, but a cons chain
/// derives as `Cons(ConsCell { car: .., cdr: Cons(ConsCell { .. } ) })`,
/// nested once per element -- unreadable in the echo area, which is where
/// most of these strings end up (`report-error`, `*Messages*`, the eval
/// minibuffer). Everything the reader can produce round-trips; the values
/// it cannot read back (lambdas, primitives, atoms, fibers) print in
/// `#<...>` form, following Elisp.
impl<T: LispContext> std::fmt::Debug for LispExp<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Both list shapes print the same way: the distinction between
            // a syntax node and a data list is ours, not the reader's.
            LispExp::Form(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{:?}", item)?;
                }
                write!(f, ")")
            }

            LispExp::Cons(_) => {
                let (items, tail) = self.split_list();
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{:?}", item)?;
                }
                if !tail.is_nil() {
                    write!(f, " . {:?}", tail)?;
                }
                write!(f, ")")
            }

            LispExp::Vector(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{:?}", item)?;
                }
                write!(f, "]")
            }

            LispExp::Map(map) => {
                write!(f, "{{")?;
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{} {:?}", key, value)?;
                }
                write!(f, "}}")
            }

            // Every number is an `f64`, but `(+ 1 2)` should echo as `3`,
            // not `3.0`. Only values that really are integral take the
            // integer path, so `1.5` and `1e300` still print faithfully.
            LispExp::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }

            LispExp::Symbol(s) => write!(f, "{}", s),

            // Rust's string escaping is close enough to Lisp's that the
            // result reads back correctly for the escapes the lexer knows.
            LispExp::String(s) => write!(f, "{:?}", s.as_str()),

            LispExp::Lambda(lambda) => {
                write!(f, "#<lambda (")?;
                let mut first = true;
                for param in &lambda.params {
                    if !first {
                        write!(f, " ")?;
                    }
                    first = false;
                    write!(f, "{}", param)?;
                }
                if !lambda.optionals.is_empty() {
                    if !first {
                        write!(f, " ")?;
                    }
                    first = false;
                    write!(f, "&optional")?;
                    for param in &lambda.optionals {
                        write!(f, " {}", param)?;
                    }
                }
                if let Some(rest) = &lambda.rest {
                    if !first {
                        write!(f, " ")?;
                    }
                    write!(f, "&rest {}", rest)?;
                }
                write!(f, ")>")
            }

            LispExp::Primitive { .. } => write!(f, "#<primitive>"),

            // Deliberately opaque. An atom is the one mutable container in
            // the language, so it is also the one place a value can be made
            // to contain itself -- recursing here could hang the printer,
            // and taking the lock could deadlock against a writer that is
            // formatting its own contents. `deref` is how you look inside.
            LispExp::Atom(_) => write!(f, "#<atom>"),

            LispExp::Fiber(fiber) => match fiber.0.try_read() {
                Ok(state) if state.is_done => write!(f, "#<fiber done>"),
                Ok(_) => write!(f, "#<fiber>"),
                Err(_) => write!(f, "#<fiber running>"),
            },
        }
    }
}

impl<T: LispContext> LispExp<T> {
    pub fn symbol(value: String) -> LispExp<T> {
        LispExp::Symbol(Arc::new(value))
    }

    pub fn nil() -> LispExp<T> {
        LispExp::symbol("nil".into())
    }

    pub fn t() -> LispExp<T> {
        LispExp::symbol("t".into())
    }

    pub fn boolean(value: bool) -> LispExp<T> {
        if value { Self::t() } else { Self::nil() }
    }

    pub fn is_nil(&self) -> bool {
        match self {
            LispExp::Symbol(s) => s.as_str() == "nil",
            LispExp::Form(l) => l.is_empty(),
            _ => false,
        }
    }

    pub fn is_truthy(&self) -> bool {
        !self.is_nil()
    }

    pub fn string(value: String) -> LispExp<T> {
        LispExp::String(Arc::new(value))
    }

    pub fn number(value: f64) -> LispExp<T> {
        LispExp::Number(value)
    }

    pub fn cons(car: LispExp<T>, cdr: LispExp<T>) -> LispExp<T> {
        LispExp::Cons(Arc::new(ConsCell { car, cdr }))
    }

    /// Fold a vector into a proper list, right to left.
    pub fn proper_list(items: Vec<LispExp<T>>) -> LispExp<T> {
        items
            .into_iter()
            .rev()
            .fold(LispExp::nil(), |cdr, car| LispExp::cons(car, cdr))
    }

    /// Fold a vector into a chain ending in an arbitrary tail.
    pub fn improper_list(items: Vec<LispExp<T>>, tail: LispExp<T>) -> LispExp<T> {
        items
            .into_iter()
            .rev()
            .fold(tail, |cdr, car| LispExp::cons(car, cdr))
    }

    /// Walk a cons chain, yielding each `car`. Stops at any non-`Cons`
    /// cdr, so it silently treats an improper list as its proper prefix —
    /// callers that care use `split_list` below.
    pub fn iter(&self) -> ConsIter<T> {
        ConsIter {
            cursor: self.clone(),
        }
    }

    /// Collect a chain into `(elements, final_tail)`. The tail is `nil`
    /// for a proper list.
    pub fn split_list(&self) -> (Vec<LispExp<T>>, LispExp<T>) {
        let mut items = Vec::new();
        let mut cursor = self.clone();
        while let LispExp::Cons(cell) = cursor {
            items.push(cell.car.clone());
            cursor = cell.cdr.clone();
        }
        (items, cursor)
    }

    /// Build a *syntax* node. Reserved for the reader, macro expansion and
    /// `data_to_form`; primitives that return a list to Lisp want
    /// `proper_list` instead.
    pub fn form(value: Vec<LispExp<T>>) -> LispExp<T> {
        LispExp::Form(Arc::new(value))
    }

    pub fn vec(value: Vec<LispExp<T>>) -> LispExp<T> {
        LispExp::Vector(Arc::new(value))
    }

    pub fn map(value: HashMap<String, LispExp<T>>) -> LispExp<T> {
        LispExp::Map(Arc::new(value))
    }

    pub fn lambda(value: Lambda<T>) -> LispExp<T> {
        LispExp::Lambda(Arc::new(value))
    }

    pub fn fiber(value: FiberState<T>) -> LispExp<T> {
        LispExp::Fiber(SharedFiber(Arc::new(RwLock::new(value))))
    }

    /// Build a `Primitive` value from a native function pointer and an
    /// optional docstring. DOC accepts either a `&'static str` -- the
    /// common case for primitives whose documentation is a literal sitting
    /// right next to the function, which costs no allocation -- or an owned
    /// `String`, for documentation that only exists at runtime (e.g. loaded
    /// alongside a primitive from a dynamic library). Either way the
    /// resulting `Primitive` stays cheap to `Clone`: only the `Arc`'s
    /// refcount is bumped, never the string itself.
    pub fn primitive(pointer: LispPrimitive<T>, doc: Option<Cow<'static, str>>) -> LispExp<T> {
        LispExp::Primitive {
            pointer,
            doc: Arc::new(doc.unwrap_or(Cow::Borrowed("No documentation provided."))),
        }
    }
}
